//! Eğlence komutları: kullanıcılar arası mini oyunlar.
//!
//! - `/uyum` ve kullanıcıya sağ tık → **Apps → Uyumumuz**: iki kişinin "kader" uyumu. Skor kullanıcı
//!   kimliklerinden deterministik üretilir (kaç kez sorulsa da aynı çıkar, sıra fark etmez).
//!   Kartın altındaki kullanıcı seçici ile tıklayan herkes kendi uyumunu dener.
//! - `/düello`: gizli seçimli Taş · Kağıt · Makas; ilk iki turu alan kazanır. Bota meydan
//!   okunursa bot rakip olarak oynar.
//! - `/sayı-tahmini`: bot bir sayı tutar; herkes butondan açılan **modal** ile tahmin eder,
//!   kart daralan aralığı ve sıcak/soğuk ipuçlarını gösterir.
//! - `/8top`: sihirli 8 top.
//!
//! Kartlar Components V2 (Container, Media Gallery, Separator) ile ham JSON olarak kurulur;
//! serenity 0.12 bu bileşenler için builder sunmaz. Etkileşim yanıtı doğrudan
//! `create_interaction_response` ile verilir, bu yüzden komutlar DM ve user-install
//! bağlamlarında da çalışır (bot kanal üyesi olmak zorunda değildir).
//!
//! Oyun durumu bellekte tutulur ve [`GAME_TTL`] sonra düşer; süreç yeniden başlarsa açık
//! oyunların butonları "süresi doldu" yanıtı verir. Kartlardaki etiketler ping atmaz
//! (`allowed_mentions.parse = []`); yalnızca düello daveti rakibe bildirim gönderir.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use serenity::all::{
    ActionRowComponent, CommandDataOptionValue, CommandDataResolved, CommandInteraction,
    CommandOptionType, CommandType, ComponentInteraction, ComponentInteractionDataKind, Context,
    CreateCommand, CreateCommandOption, InstallationContext, InteractionContext, InteractionId,
    ModalInteraction, User, UserId,
};
use tracing::warn;

use crate::converters::builtin::render::escape_markdown;
use crate::converters::builtin::v2::COMPONENTS_V2_FLAG;
use crate::utils::truncate_chars;

/// Uyum sohbet komutu.
pub const COMPAT_COMMAND: &str = "uyum";
/// Kullanıcıya sağ tık → Apps menüsündeki uyum komutu.
pub const COMPAT_USER_COMMAND: &str = "Uyumumuz";
/// Taş · Kağıt · Makas düellosu.
pub const DUEL_COMMAND: &str = "düello";
/// Sayı tahmini oyunu.
pub const GUESS_COMMAND: &str = "sayı-tahmini";
/// Sihirli 8 top.
pub const EIGHT_BALL_COMMAND: &str = "8top";

/// Bu modülün buton/seçici/modal `custom_id` öneki.
pub const CUSTOM_ID_PREFIX: &str = "fun:";

const PERSON_OPTION: &str = "kişi";
const SECOND_OPTION: &str = "ikinci";
const RIVAL_OPTION: &str = "rakip";
const QUESTION_OPTION: &str = "soru";
const DIFFICULTY_OPTION: &str = "zorluk";
const GUESS_INPUT: &str = "tahmin";

/// Açık oyunların bellekte kalma süresi.
const GAME_TTL: Duration = Duration::from_secs(15 * 60);
/// Oyun türü başına aynı anda tutulan en fazla oyun (bellek sınırı).
const MAX_GAMES: usize = 500;
/// Düello turu üst sınırı: arka arkaya beraberliklerde oyun sonsuza kadar sürmesin.
const MAX_DUEL_ROUNDS: usize = 9;
/// Sayı tahmini kartında gösterilen son tahmin sayısı.
const GUESS_LOG_LINES: usize = 6;

// Etkileşim yanıt türleri
const RESPONSE_MESSAGE: u8 = 4;
const RESPONSE_UPDATE: u8 = 7;
const RESPONSE_MODAL: u8 = 9;
const EPHEMERAL_FLAG: u64 = 1 << 6;

// Buton stilleri
const STYLE_PRIMARY: u8 = 1;
const STYLE_SECONDARY: u8 = 2;
const STYLE_SUCCESS: u8 = 3;
const STYLE_DANGER: u8 = 4;

/// Discord'a gönderilen eğlence komutu tanımları.
///
/// Dönüştürme komutlarıyla aynı nedenle `integration_types` ve `contexts` açıkça gönderilir
/// (bkz. [`super::definitions`]): aksi halde komutlar DM'lerde görünmez.
pub fn definitions() -> Vec<CreateCommand> {
    vec![
        everywhere(CreateCommand::new(COMPAT_COMMAND))
            .description("İki kişinin kaderde yazan uyumunu ölçer 💞")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::User,
                    PERSON_OPTION,
                    "Kiminle uyumuna bakılsın?",
                )
                .required(true),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::User,
                    SECOND_OPTION,
                    "Başka iki kişiyi eşleştirmek için ikinci kişi (boşsa sen)",
                )
                .required(false),
            ),
        everywhere(CreateCommand::new(COMPAT_USER_COMMAND))
            .kind(CommandType::User)
            // Discord, USER/MESSAGE komutlarında açıklamayı boş string olarak bekler.
            .description(""),
        everywhere(CreateCommand::new(DUEL_COMMAND))
            .description("Birine Taş · Kağıt · Makas düellosu teklif et ⚔️")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::User,
                    RIVAL_OPTION,
                    "Meydan okuduğun kişi",
                )
                .required(true),
            ),
        everywhere(CreateCommand::new(GUESS_COMMAND))
            .description("Tuttuğum sayıyı ilk bilen kazanır 🎯")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    DIFFICULTY_OPTION,
                    "Sayı aralığı",
                )
                .required(false)
                .add_int_choice("Kolay (1–100)", 100)
                .add_int_choice("Orta (1–500)", 500)
                .add_int_choice("Zor (1–1000)", 1000),
            ),
        everywhere(CreateCommand::new(EIGHT_BALL_COMMAND))
            .description("Sihirli 8 topa bir soru sor 🎱")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    QUESTION_OPTION,
                    "Evet/hayır sorusu",
                )
                .required(true)
                .max_length(200),
            ),
    ]
}

fn everywhere(command: CreateCommand) -> CreateCommand {
    command
        .add_integration_type(InstallationContext::Guild)
        .add_integration_type(InstallationContext::User)
        .add_context(InteractionContext::Guild)
        .add_context(InteractionContext::BotDm)
        .add_context(InteractionContext::PrivateChannel)
}

/// Bu komut etkileşimi eğlence modülüne mi ait?
pub fn is_fun_command(command: &CommandInteraction) -> bool {
    match command.data.kind {
        CommandType::ChatInput => matches!(
            command.data.name.as_str(),
            COMPAT_COMMAND | DUEL_COMMAND | GUESS_COMMAND | EIGHT_BALL_COMMAND
        ),
        CommandType::User => command.data.name == COMPAT_USER_COMMAND,
        _ => false,
    }
}

/// Slash ve kullanıcı komutlarını yanıtlar.
pub async fn handle_command(ctx: &Context, command: &CommandInteraction) {
    let invoker = Profile::of(
        &command.user,
        command.member.as_ref().and_then(|m| m.nick.as_deref()),
    );
    let resolved = &command.data.resolved;

    let response = match (command.data.kind, command.data.name.as_str()) {
        (CommandType::User, _) => {
            let target = command
                .data
                .target_id
                .and_then(|id| resolved_profile(resolved, UserId::new(id.get())));
            match target {
                Some(target) => compat_response(&invoker, &target),
                None => notice("Bu kişiyi bulamadım 🤷"),
            }
        }
        (_, COMPAT_COMMAND) => {
            let first =
                user_option(command, PERSON_OPTION).and_then(|id| resolved_profile(resolved, id));
            let second =
                user_option(command, SECOND_OPTION).and_then(|id| resolved_profile(resolved, id));
            match (first, second) {
                (Some(first), Some(second)) => compat_response(&first, &second),
                (Some(first), None) => compat_response(&invoker, &first),
                _ => notice("Kiminle uyumuna bakayım? `/uyum kişi:@biri`"),
            }
        }
        (_, DUEL_COMMAND) => {
            let bot_id = ctx.cache.current_user().id;
            match user_option(command, RIVAL_OPTION) {
                Some(rival) => start_duel(command.id, invoker.id, rival, resolved, bot_id),
                None => notice("Kime meydan okuyorsun? `/düello rakip:@biri`"),
            }
        }
        (_, GUESS_COMMAND) => {
            let max = integer_option(command, DIFFICULTY_OPTION)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| matches!(value, 100 | 500 | 1000))
                .unwrap_or(100);
            start_guessing(command.id.get(), invoker.id, max)
        }
        (_, EIGHT_BALL_COMMAND) => {
            let question = string_option(command, QUESTION_OPTION).unwrap_or_default();
            eight_ball_response(invoker.id, question)
        }
        _ => return,
    };

    respond(ctx, command.id, &command.token, &response).await;
}

/// `fun:` önekli buton ve seçici etkileşimlerini yanıtlar.
pub async fn handle_component(ctx: &Context, component: &ComponentInteraction) {
    let parts: Vec<&str> = component.data.custom_id.split(':').collect();
    let clicker = component.user.id;

    let (response, followup) = match parts.as_slice() {
        ["fun", "uyum", "pick"] => (compat_pick(component), None),
        ["fun", "tkm", "rematch", a, b] => {
            let players = [a, b].map(|id| id.parse().ok().map(UserId::new));
            match players {
                [Some(a), Some(b)] if clicker == a || clicker == b => {
                    let rival = if clicker == a { b } else { a };
                    let bot_rival = rival == ctx.cache.current_user().id;
                    (
                        new_duel(component.id.get(), clicker, rival, bot_rival),
                        None,
                    )
                }
                _ => (
                    notice("Rövanşı yalnızca düellodaki iki kişi isteyebilir 👀"),
                    None,
                ),
            }
        }
        ["fun", "tkm", id, code] => duel_click(id, code, clicker),
        ["fun", "tahmin", "new", max] => {
            let max = max
                .parse()
                .ok()
                .filter(|max| matches!(max, 100 | 500 | 1000))
                .unwrap_or(100);
            (start_guessing(component.id.get(), clicker, max), None)
        }
        ["fun", "tahmin", id, "g"] => (guess_modal(id), None),
        ["fun", "tahmin", id, "x"] => (guess_give_up(id, clicker), None),
        _ => return,
    };

    respond(ctx, component.id, &component.token, &response).await;
    if let Some(content) = followup {
        let body = json!({ "content": content, "flags": EPHEMERAL_FLAG });
        if let Err(err) = ctx
            .http
            .create_followup_message(&component.token, &body, Vec::new())
            .await
        {
            warn!(interaction = %component.id, "Eğlence takip mesajı gönderilemedi: {err:?}");
        }
    }
}

/// `fun:` önekli modal gönderimlerini yanıtlar.
pub async fn handle_modal(ctx: &Context, modal: &ModalInteraction) {
    let parts: Vec<&str> = modal.data.custom_id.split(':').collect();
    let ["fun", "tahmin", id, "m"] = parts.as_slice() else {
        return;
    };
    let value = modal
        .data
        .components
        .iter()
        .flat_map(|row| row.components.iter())
        .find_map(|component| match component {
            ActionRowComponent::InputText(input) if input.custom_id == GUESS_INPUT => {
                input.value.as_deref()
            }
            _ => None,
        })
        .unwrap_or_default();
    let response = guess_submit(id, modal.user.id, value);
    respond(ctx, modal.id, &modal.token, &response).await;
}

async fn respond(ctx: &Context, id: InteractionId, token: &str, body: &Value) {
    if let Err(err) = ctx
        .http
        .create_interaction_response(id, token, body, Vec::new())
        .await
    {
        warn!(interaction = %id, "Eğlence etkileşimi yanıtlanamadı: {err:?}");
    }
}

// ─── Uyum ─────────────────────────────────────────────────────────────────────

/// Kartta gösterilen kullanıcı bilgisi.
struct Profile {
    id: UserId,
    name: String,
    avatar: String,
    bot: bool,
}

impl Profile {
    fn of(user: &User, nick: Option<&str>) -> Self {
        Self {
            id: user.id,
            name: nick.unwrap_or(user.display_name()).to_string(),
            avatar: user.face(),
            bot: user.bot,
        }
    }
}

fn resolved_profile(resolved: &CommandDataResolved, id: UserId) -> Option<Profile> {
    let user = resolved.users.get(&id)?;
    let nick = resolved.members.get(&id).and_then(|m| m.nick.as_deref());
    Some(Profile::of(user, nick))
}

/// Uyum alt başlıkları; her biri genel skorun etrafında dalgalanır.
const TRAITS: [&str; 5] = [
    "💬 Sohbet",
    "😂 Mizah",
    "🧭 Macera",
    "🤝 Güven",
    "🔥 Kıvılcım",
];

struct Compatibility {
    score: u8,
    traits: [u8; TRAITS.len()],
}

/// İki kişinin uyumu: simetrik (sıra fark etmez) ve deterministik.
fn compatibility(a: UserId, b: UserId) -> Compatibility {
    if a == b {
        return Compatibility {
            score: 100,
            traits: [100; TRAITS.len()],
        };
    }
    let (low, high) = if a < b {
        (a.get(), b.get())
    } else {
        (b.get(), a.get())
    };
    let seed = splitmix64(low ^ splitmix64(high ^ 0x6275_7967_656b_5f21));
    let score = (seed % 101) as u8;
    let mut traits = [0; TRAITS.len()];
    for (index, value) in traits.iter_mut().enumerate() {
        let wobble = (splitmix64(seed.wrapping_add(index as u64 + 1)) % 41) as i16 - 20;
        *value = (i16::from(score) + wobble).clamp(0, 100) as u8;
    }
    Compatibility { score, traits }
}

/// Skora göre karar cümlesi ve kart rengi.
fn verdict(score: u8) -> (&'static str, u32) {
    match score {
        100 => ("💍 Ruh ikizi! Nikah memuru kapıda bekliyor.", 0xFF2E88),
        90..=99 => (
            "💖 Yıldızlar aynı anda yanıp sönüyor. Bu iş olur!",
            0xFF4FA3,
        ),
        75..=89 => ("💗 Harika uyum; kıvılcımlar uçuşuyor.", 0xFF6B9A),
        55..=74 => (
            "😊 Güzel bir potansiyel var, bir kahve içmeye değer.",
            0xFFB347,
        ),
        35..=54 => ("🤔 Biraz emek ister ama imkânsız değil.", 0xF4D35E),
        15..=34 => ("🧊 Buzlar erimeden bahar gelir mi bilinmez…", 0x6EC6FF),
        _ => (
            "💀 Evren net bir şekilde \"hayır\" diyor. Arkadaş kalalım.",
            0x8B8B9E,
        ),
    }
}

/// İki adın yarılarından "ship adı" üretir: `Ayşe` + `Mehmet` → `Aymet`.
fn ship_name(a: &str, b: &str) -> String {
    let a: Vec<char> = a.chars().filter(|c| c.is_alphanumeric()).collect();
    let b: Vec<char> = b.chars().filter(|c| c.is_alphanumeric()).collect();
    if a.is_empty() || b.is_empty() {
        return "Bilinmeyen Ship".to_string();
    }
    let head: String = a[..a.len().div_ceil(2)].iter().collect();
    let tail: String = b[b.len() / 2..].iter().collect::<String>().to_lowercase();
    truncate_chars(&format!("{head}{tail}"), 32)
}

fn compat_response(a: &Profile, b: &Profile) -> Value {
    message(RESPONSE_MESSAGE, compat_card(a, b))
}

fn compat_card(a: &Profile, b: &Profile) -> Value {
    let result = compatibility(a.id, b.id);
    let (line, accent) = verdict(result.score);
    let name_a = escape_markdown(&a.name);
    let name_b = escape_markdown(&b.name);

    let mut children = Vec::new();
    if a.id == b.id {
        children.push(text(format!(
            "## 🪞 {name_a} × {name_a}\n-# Kendini sevmek her şeyin başı."
        )));
        children.push(gallery(&[a.avatar.as_str()]));
    } else {
        children.push(text(format!(
            "## 💞 {name_a} × {name_b}\n-# Ship adı: **{}**",
            escape_markdown(&ship_name(&a.name, &b.name))
        )));
        children.push(gallery(&[a.avatar.as_str(), b.avatar.as_str()]));
    }
    children.push(text(format!(
        "### {} **%{}**\n{line}",
        bar(result.score, 12),
        result.score
    )));
    children.push(separator(true));
    let traits = TRAITS
        .iter()
        .zip(result.traits)
        .map(|(name, value)| format!("{name} `{}` %{value}", bar(value, 8)))
        .collect::<Vec<_>>()
        .join("\n");
    children.push(text(traits));
    if a.bot || b.bot {
        children.push(text("-# 🤖 Not: Botlar sevgiyi ikili sistemde ifade eder: 01101100 01101111 01110110 01100101"));
    }
    children.push(text(format!(
        "-# 🔮 <@{}> & <@{}> • Bu skor yıldızlarla mühürlendi; kaç kez sorsan da değişmez.",
        a.id, b.id
    )));

    let picker = json!({
        "type": 1,
        "components": [{
            "type": 5,
            "custom_id": "fun:uyum:pick",
            "placeholder": "💘 Sen de birini dene…",
            "min_values": 1,
            "max_values": 1
        }]
    });
    v2_message(vec![container(accent, children), picker])
}

/// Kullanıcı seçiciden gelen uyum isteği: tıklayan kişi × seçilen kişi, yeni mesaj olarak.
fn compat_pick(component: &ComponentInteraction) -> Value {
    let ComponentInteractionDataKind::UserSelect { values } = &component.data.kind else {
        return notice("Bir kişi seçmelisin 💘");
    };
    let Some(target) = values
        .first()
        .and_then(|id| resolved_profile(&component.data.resolved, *id))
    else {
        return notice("Seçtiğin kişiyi bulamadım 🤷");
    };
    let clicker = Profile::of(
        &component.user,
        component.member.as_ref().and_then(|m| m.nick.as_deref()),
    );
    compat_response(&clicker, &target)
}

// ─── Düello (Taş · Kağıt · Makas) ────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    Rock,
    Paper,
    Scissors,
}

impl Move {
    const ALL: [Move; 3] = [Move::Rock, Move::Paper, Move::Scissors];

    fn code(self) -> &'static str {
        match self {
            Move::Rock => "r",
            Move::Paper => "p",
            Move::Scissors => "s",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mv| mv.code() == code)
    }

    fn emoji(self) -> &'static str {
        match self {
            Move::Rock => "🪨",
            Move::Paper => "📄",
            Move::Scissors => "✂️",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Move::Rock => "Taş",
            Move::Paper => "Kağıt",
            Move::Scissors => "Makas",
        }
    }

    fn beats(self, other: Move) -> bool {
        matches!(
            (self, other),
            (Move::Rock, Move::Scissors)
                | (Move::Paper, Move::Rock)
                | (Move::Scissors, Move::Paper)
        )
    }

    /// Kazanan hamlenin anlatımı ("Taş makası ezdi!").
    fn flavor(self) -> &'static str {
        match self {
            Move::Rock => "🪨 Taş, makası ezdi!",
            Move::Paper => "📄 Kağıt, taşı sardı!",
            Move::Scissors => "✂️ Makas, kağıdı kesti!",
        }
    }

    fn random() -> Self {
        Self::ALL[(random_u64() % 3) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuelResult {
    Winner { index: usize, forfeit: bool },
    Draw,
}

struct Duel {
    players: [UserId; 2],
    /// İkinci oyuncu bu bot mu? (bot her turun başında kendi seçimini yapar)
    bot_rival: bool,
    picks: [Option<Move>; 2],
    wins: [u8; 2],
    rounds: Vec<(Move, Move)>,
    result: Option<DuelResult>,
    expires_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum PlayOutcome {
    AlreadyPicked,
    Waiting,
    RoundOver,
}

impl Duel {
    fn new(challenger: UserId, rival: UserId, bot_rival: bool) -> Self {
        let mut duel = Self {
            players: [challenger, rival],
            bot_rival,
            picks: [None, None],
            wins: [0, 0],
            rounds: Vec::new(),
            result: None,
            expires_at: unix_now() + GAME_TTL.as_secs(),
        };
        duel.bot_pick();
        duel
    }

    fn bot_pick(&mut self) {
        if self.bot_rival {
            self.picks[1] = Some(Move::random());
        }
    }

    fn index_of(&self, user: UserId) -> Option<usize> {
        self.players.iter().position(|player| *player == user)
    }

    fn play(&mut self, index: usize, mv: Move) -> PlayOutcome {
        if self.picks[index].is_some() {
            return PlayOutcome::AlreadyPicked;
        }
        self.picks[index] = Some(mv);
        let [Some(first), Some(second)] = self.picks else {
            return PlayOutcome::Waiting;
        };

        self.rounds.push((first, second));
        self.picks = [None, None];
        if first.beats(second) {
            self.wins[0] += 1;
        } else if second.beats(first) {
            self.wins[1] += 1;
        }
        if let Some(index) = self.wins.iter().position(|wins| *wins >= 2) {
            self.result = Some(DuelResult::Winner {
                index,
                forfeit: false,
            });
        } else if self.rounds.len() >= MAX_DUEL_ROUNDS {
            self.result = Some(match self.wins[0].cmp(&self.wins[1]) {
                std::cmp::Ordering::Greater => DuelResult::Winner {
                    index: 0,
                    forfeit: false,
                },
                std::cmp::Ordering::Less => DuelResult::Winner {
                    index: 1,
                    forfeit: false,
                },
                std::cmp::Ordering::Equal => DuelResult::Draw,
            });
        } else {
            self.bot_pick();
        }
        PlayOutcome::RoundOver
    }

    fn forfeit(&mut self, index: usize) {
        self.result = Some(DuelResult::Winner {
            index: 1 - index,
            forfeit: true,
        });
    }
}

static DUELS: LazyLock<GameStore<Duel>> = LazyLock::new(GameStore::new);

fn start_duel(
    id: InteractionId,
    challenger: UserId,
    rival: UserId,
    resolved: &CommandDataResolved,
    bot_id: UserId,
) -> Value {
    if rival == challenger {
        return notice("Kendinle düello mu? Aynayla oynanan her oyun berabere biter 🪞");
    }
    let rival_is_bot = resolved.users.get(&rival).is_some_and(|user| user.bot);
    if rival_is_bot && rival != bot_id {
        return notice("Bu bot düello davetini kabul etmiyor 🤖 Bana meydan okuyabilirsin ama!");
    }
    new_duel(id.get(), challenger, rival, rival == bot_id)
}

fn new_duel(id: u64, challenger: UserId, rival: UserId, bot_rival: bool) -> Value {
    let duel = Duel::new(challenger, rival, bot_rival);
    let mut data = duel_card(id, &duel);
    // Yalnızca davet rakibe bildirim gönderir; tur güncellemeleri sessizdir.
    if !bot_rival {
        data["allowed_mentions"] = allowed_users(&[rival]);
    }
    DUELS.insert(id, duel);
    message(RESPONSE_MESSAGE, data)
}

/// Düello butonu: (yanıt, tıklayana gizli takip mesajı).
fn duel_click(id: &str, code: &str, clicker: UserId) -> (Value, Option<String>) {
    let Ok(id) = id.parse::<u64>() else {
        return (notice("Bu düello bulunamadı."), None);
    };
    let outcome = DUELS.with(id, |duel| {
        let Some(index) = duel.index_of(clicker) else {
            return Err("Bu düello senin değil 👀 `/düello` ile kendininkini başlat.");
        };
        if duel.bot_rival && index == 1 {
            return Err("Bot kendi adına oynar 🤖");
        }
        if code == "x" {
            duel.forfeit(index);
            return Ok((duel_card(id, duel), None, true));
        }
        let Some(mv) = Move::from_code(code) else {
            return Err("Geçersiz hamle.");
        };
        match duel.play(index, mv) {
            PlayOutcome::AlreadyPicked => Err("Bu tur seçimini zaten yaptın 🤫 Rakibini bekle."),
            PlayOutcome::Waiting => Ok((
                duel_card(id, duel),
                Some(format!(
                    "🤫 Seçimin: {} **{}**. Rakibin bekleniyor…",
                    mv.emoji(),
                    mv.label()
                )),
                false,
            )),
            PlayOutcome::RoundOver => Ok((duel_card(id, duel), None, duel.result.is_some())),
        }
    });

    match outcome {
        None => (
            notice("Bu düellonun süresi doldu ⌛ `/düello` ile yenisini başlat."),
            None,
        ),
        Some(Err(text)) => (notice(text), None),
        Some(Ok((card, followup, finished))) => {
            if finished {
                DUELS.remove(id);
            }
            (message(RESPONSE_UPDATE, card), followup)
        }
    }
}

fn duel_card(id: u64, duel: &Duel) -> Value {
    let [a, b] = duel.players;
    let mut children = vec![
        text(format!(
            "## ⚔️ Taş · Kağıt · Makas\n<@{a}> **vs** <@{b}>\n-# İlk iki turu kazanan düelloyu alır."
        )),
        separator(true),
    ];

    let status = |index: usize| {
        if duel.picks[index].is_some() {
            "✅ seçti"
        } else {
            "⏳ düşünüyor…"
        }
    };
    let mut board = format!("### Skor: {} – {}", duel.wins[0], duel.wins[1]);
    if duel.result.is_none() {
        board.push_str(&format!("\n<@{a}> {}\n<@{b}> {}", status(0), status(1)));
    }
    children.push(text(board));

    if !duel.rounds.is_empty() {
        let first = duel.rounds.len().saturating_sub(5);
        let lines = duel.rounds[first..]
            .iter()
            .enumerate()
            .map(|(offset, (x, y))| {
                let outcome = if x.beats(*y) {
                    format!("<@{a}> · {}", x.flavor())
                } else if y.beats(*x) {
                    format!("<@{b}> · {}", y.flavor())
                } else {
                    "🤝 Berabere".to_string()
                };
                format!(
                    "`{}.` {} vs {} → {outcome}",
                    first + offset + 1,
                    x.emoji(),
                    y.emoji()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        children.push(text(format!("**Turlar**\n{lines}")));
    }

    let mut accent = 0x5865F2;
    let mut rows = Vec::new();
    match duel.result {
        Some(result) => {
            let line = match result {
                DuelResult::Winner {
                    index,
                    forfeit: false,
                } => {
                    accent = 0x57F287;
                    format!("## 🏆 <@{}> düelloyu kazandı!", duel.players[index])
                }
                DuelResult::Winner {
                    index,
                    forfeit: true,
                } => {
                    accent = 0xFEE75C;
                    format!(
                        "## 🏳️ <@{}> pes etti, <@{}> kazandı!",
                        duel.players[1 - index],
                        duel.players[index]
                    )
                }
                DuelResult::Draw => {
                    accent = 0x99AAB5;
                    "## 🤝 Berabere! Kimse pes etmedi, kimse kazanmadı.".to_string()
                }
            };
            children.push(separator(true));
            children.push(text(line));
            rows.push(row(vec![button(
                STYLE_PRIMARY,
                "Rövanş",
                "🔁",
                &format!("fun:tkm:rematch:{a}:{b}"),
            )]));
        }
        None => {
            children.push(text(format!(
                "-# Seçimler gizlidir; ikiniz de seçince tur açılır • <t:{}:R> sona erer",
                duel.expires_at
            )));
            let mut buttons: Vec<Value> = Move::ALL
                .into_iter()
                .map(|mv| {
                    button(
                        STYLE_SECONDARY,
                        mv.label(),
                        mv.emoji(),
                        &format!("fun:tkm:{id}:{}", mv.code()),
                    )
                })
                .collect();
            buttons.push(button(
                STYLE_DANGER,
                "Pes et",
                "🏳️",
                &format!("fun:tkm:{id}:x"),
            ));
            rows.push(row(buttons));
        }
    }

    let mut components = vec![container(accent, children)];
    components.extend(rows);
    v2_message(components)
}

// ─── Sayı tahmini ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hint {
    Higher,
    Lower,
}

struct Guessing {
    secret: u32,
    max: u32,
    host: UserId,
    /// Şu ana kadarki tahminlerle daralan olası aralık.
    low: u32,
    high: u32,
    log: Vec<(UserId, u32, Hint)>,
    attempts: u32,
    winner: Option<UserId>,
    gave_up: bool,
    expires_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum GuessOutcome {
    OutOfRange,
    Correct,
    Wrong(Hint),
}

impl Guessing {
    fn new(host: UserId, max: u32, secret: u32) -> Self {
        Self {
            secret,
            max,
            host,
            low: 1,
            high: max,
            log: Vec::new(),
            attempts: 0,
            winner: None,
            gave_up: false,
            expires_at: unix_now() + GAME_TTL.as_secs(),
        }
    }

    fn finished(&self) -> bool {
        self.winner.is_some() || self.gave_up
    }

    fn guess(&mut self, user: UserId, value: u32) -> GuessOutcome {
        if !(1..=self.max).contains(&value) {
            return GuessOutcome::OutOfRange;
        }
        self.attempts += 1;
        if value == self.secret {
            self.winner = Some(user);
            self.low = value;
            self.high = value;
            return GuessOutcome::Correct;
        }
        let hint = if value < self.secret {
            self.low = self.low.max(value + 1);
            Hint::Higher
        } else {
            self.high = self.high.min(value - 1);
            Hint::Lower
        };
        self.log.push((user, value, hint));
        GuessOutcome::Wrong(hint)
    }

    /// Tahminin gizli sayıya yakınlığı (aralık genişliğine göre).
    fn heat(&self, value: u32) -> &'static str {
        let distance = value.abs_diff(self.secret) * 100 / self.max;
        match distance {
            0..=2 => "🔥 kaynıyor",
            3..=6 => "♨️ sıcak",
            7..=15 => "🌤️ ılık",
            16..=30 => "🧊 soğuk",
            _ => "🥶 buz gibi",
        }
    }
}

static GUESSES: LazyLock<GameStore<Guessing>> = LazyLock::new(GameStore::new);

fn start_guessing(id: u64, host: UserId, max: u32) -> Value {
    let secret = (random_u64() % u64::from(max)) as u32 + 1;
    let game = Guessing::new(host, max, secret);
    let data = guess_card(id, &game);
    GUESSES.insert(id, game);
    message(RESPONSE_MESSAGE, data)
}

fn guess_modal(id: &str) -> Value {
    let Some(max) = id
        .parse::<u64>()
        .ok()
        .and_then(|id| GUESSES.with(id, |game| (!game.finished()).then_some(game.max)))
        .flatten()
    else {
        return notice("Bu oyun bitti ya da süresi doldu ⌛ `/sayı-tahmini` ile yenisini başlat.");
    };
    json!({
        "type": RESPONSE_MODAL,
        "data": {
            "custom_id": format!("fun:tahmin:{id}:m"),
            "title": "🎯 Tahminin ne?",
            "components": [{
                "type": 1,
                "components": [{
                    "type": 4,
                    "custom_id": GUESS_INPUT,
                    "style": 1,
                    "label": format!("1 ile {max} arasında bir sayı"),
                    "placeholder": format!("örn. {}", max / 2),
                    "min_length": 1,
                    "max_length": 4,
                    "required": true
                }]
            }]
        }
    })
}

fn guess_submit(id: &str, user: UserId, value: &str) -> Value {
    let Ok(id) = id.parse::<u64>() else {
        return notice("Bu oyun bulunamadı.");
    };
    let Ok(value) = value.trim().parse::<u32>() else {
        return notice("Bu bir sayı değil 🙃");
    };
    let outcome = GUESSES.with(id, |game| {
        if game.finished() {
            return Err("Bu oyun çoktan bitti 🏁".to_string());
        }
        match game.guess(user, value) {
            GuessOutcome::OutOfRange => Err(format!("Sayı 1 ile {} arasında olmalı.", game.max)),
            outcome => Ok((guess_card(id, game), outcome == GuessOutcome::Correct)),
        }
    });
    match outcome {
        None => notice("Bu oyunun süresi doldu ⌛ `/sayı-tahmini` ile yenisini başlat."),
        Some(Err(text)) => notice(&text),
        Some(Ok((card, finished))) => {
            if finished {
                GUESSES.remove(id);
            }
            message(RESPONSE_UPDATE, card)
        }
    }
}

fn guess_give_up(id: &str, user: UserId) -> Value {
    let Ok(id) = id.parse::<u64>() else {
        return notice("Bu oyun bulunamadı.");
    };
    let outcome = GUESSES.with(id, |game| {
        if game.host != user {
            return Err("Yalnızca oyunu başlatan kişi pes edebilir 🙅");
        }
        game.gave_up = true;
        Ok(guess_card(id, game))
    });
    match outcome {
        None => notice("Bu oyunun süresi doldu ⌛"),
        Some(Err(text)) => notice(text),
        Some(Ok(card)) => {
            GUESSES.remove(id);
            message(RESPONSE_UPDATE, card)
        }
    }
}

fn guess_card(id: u64, game: &Guessing) -> Value {
    let mut children = vec![text(format!(
        "## 🎯 Sayı Tahmini\n<@{}> oyunu başlattı: **1–{}** arasında bir sayı tuttum 😏\n-# Herkes tahmin edebilir; ilk bilen kazanır.",
        game.host, game.max
    ))];
    children.push(separator(true));

    let mut accent = 0xF1C40F;
    let mut rows = Vec::new();
    if let Some(winner) = game.winner {
        accent = 0x57F287;
        children.push(text(format!(
            "## 🎉 <@{winner}> bildi!\nSayı **{}** idi • toplam **{}** tahmin",
            game.secret, game.attempts
        )));
    } else if game.gave_up {
        accent = 0x99AAB5;
        children.push(text(format!(
            "## 🏳️ Oyun bitti\nSayı **{}** idi. Bir dahaki sefere!",
            game.secret
        )));
    } else {
        children.push(text(format!(
            "### Aralık: `{}` – `{}`\n`{}`",
            game.low,
            game.high,
            range_bar(game.low, game.high, game.max, 20)
        )));
    }

    if !game.log.is_empty() {
        let first = game.log.len().saturating_sub(GUESS_LOG_LINES);
        let lines = game.log[first..]
            .iter()
            .rev()
            .map(|(user, value, hint)| {
                let arrow = match hint {
                    Hint::Higher => "⬆️ daha büyük",
                    Hint::Lower => "⬇️ daha küçük",
                };
                format!("<@{user}> `{value}` {arrow} · {}", game.heat(*value))
            })
            .collect::<Vec<_>>()
            .join("\n");
        children.push(text(format!("**Son tahminler**\n{lines}")));
    }

    if game.finished() {
        rows.push(row(vec![button(
            STYLE_PRIMARY,
            "Yeni oyun",
            "🔁",
            &format!("fun:tahmin:new:{}", game.max),
        )]));
    } else {
        children.push(text(format!(
            "-# {} tahmin yapıldı • <t:{}:R> sona erer",
            game.attempts, game.expires_at
        )));
        rows.push(row(vec![
            button(
                STYLE_SUCCESS,
                "Tahmin et",
                "🎯",
                &format!("fun:tahmin:{id}:g"),
            ),
            button(STYLE_DANGER, "Pes et", "🏳️", &format!("fun:tahmin:{id}:x")),
        ]));
    }

    let mut components = vec![container(accent, children)];
    components.extend(rows);
    v2_message(components)
}

/// Olası aralığı `width` hücrelik bir çubukta gösterir: `░░▓▓▓▓▓░░░`.
fn range_bar(low: u32, high: u32, max: u32, width: u32) -> String {
    (0..width)
        .map(|cell| {
            let start = cell * max / width + 1;
            let end = (cell + 1) * max / width;
            if end >= low && start <= high {
                '▓'
            } else {
                '░'
            }
        })
        .collect()
}

// ─── Sihirli 8 top ────────────────────────────────────────────────────────────

const EIGHT_BALL: [(&str, u32); 16] = [
    ("Kesinlikle evet. ✨", 0x57F287),
    ("Hiç şüphen olmasın. 💯", 0x57F287),
    ("Yıldızlar \"evet\" diyor. 🌟", 0x57F287),
    ("Büyük ihtimalle. 👍", 0x57F287),
    ("Görünüşe göre iyi. 😌", 0x57F287),
    ("İşaretler evet'i gösteriyor. 🧭", 0x57F287),
    ("Belirsiz, tekrar sor. 🌫️", 0xFEE75C),
    ("Şimdi söylemesem daha iyi. 🤐", 0xFEE75C),
    ("Bir çay demle, sonra konuşalım. 🫖", 0xFEE75C),
    ("Konsantre ol ve tekrar sor. 🧘", 0xFEE75C),
    ("Buna güvenme. 🙅", 0xED4245),
    ("Cevabım hayır. ❌", 0xED4245),
    ("Kaynaklarım \"hayır\" diyor. 📉", 0xED4245),
    ("Hiç sanmıyorum. 🤨", 0xED4245),
    ("Rüyanda görürsün. 😴", 0xED4245),
    ("Evren güldü ve konuyu değiştirdi. 🌌", 0xED4245),
];

fn eight_ball_response(asker: UserId, question: &str) -> Value {
    let (answer, accent) = EIGHT_BALL[(random_u64() % EIGHT_BALL.len() as u64) as usize];
    let question = escape_markdown(&truncate_chars(question, 200)).replace('\n', " ");
    let card = container(
        accent,
        vec![
            text(format!("## 🎱 Sihirli 8 Top\n> {question}")),
            separator(true),
            text(format!("### {answer}")),
            text(format!("-# <@{asker}> sordu • top sallandı, kader konuştu")),
        ],
    );
    message(RESPONSE_MESSAGE, v2_message(vec![card]))
}

// ─── Ortak yardımcılar ───────────────────────────────────────────────────────

/// Bellekteki oyunlar: süresi dolanlar her erişimde temizlenir.
struct GameStore<T> {
    games: Mutex<HashMap<u64, T>>,
}

trait Expiring {
    fn expires_at(&self) -> u64;
}

impl Expiring for Duel {
    fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl Expiring for Guessing {
    fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl<T: Expiring> GameStore<T> {
    fn new() -> Self {
        Self {
            games: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, T>> {
        // Zehirli kilit (panik eden bir tıklama) oyunların tamamını kilitlemesin.
        let mut games = self.games.lock().unwrap_or_else(|err| err.into_inner());
        let now = unix_now();
        games.retain(|_, game| game.expires_at() > now);
        games
    }

    fn insert(&self, id: u64, game: T) {
        let mut games = self.lock();
        if games.len() >= MAX_GAMES
            && let Some(oldest) = games
                .iter()
                .min_by_key(|(_, game)| game.expires_at())
                .map(|(id, _)| *id)
        {
            games.remove(&oldest);
        }
        games.insert(id, game);
    }

    fn with<R>(&self, id: u64, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.lock().get_mut(&id).map(f)
    }

    fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }
}

fn user_option(command: &CommandInteraction, name: &str) -> Option<UserId> {
    command
        .data
        .options
        .iter()
        .find_map(|option| match &option.value {
            CommandDataOptionValue::User(id) if option.name == name => Some(*id),
            _ => None,
        })
}

fn integer_option(command: &CommandInteraction, name: &str) -> Option<i64> {
    command
        .data
        .options
        .iter()
        .find_map(|option| match &option.value {
            CommandDataOptionValue::Integer(value) if option.name == name => Some(*value),
            _ => None,
        })
}

fn string_option<'a>(command: &'a CommandInteraction, name: &str) -> Option<&'a str> {
    command
        .data
        .options
        .iter()
        .find_map(|option| match &option.value {
            CommandDataOptionValue::String(value) if option.name == name => Some(value.as_str()),
            _ => None,
        })
}

/// `▰▰▰▰▱▱` biçiminde yüzde çubuğu.
fn bar(percent: u8, width: usize) -> String {
    let filled = (usize::from(percent.min(100)) * width + 50) / 100;
    "▰".repeat(filled) + &"▱".repeat(width - filled)
}

fn message(kind: u8, data: Value) -> Value {
    json!({ "type": kind, "data": data })
}

/// Yalnızca tıklayana görünen düz metin yanıtı.
fn notice(content: &str) -> Value {
    message(
        RESPONSE_MESSAGE,
        json!({ "content": content, "flags": EPHEMERAL_FLAG }),
    )
}

fn v2_message(components: Vec<Value>) -> Value {
    json!({
        "flags": COMPONENTS_V2_FLAG,
        "components": components,
        "allowed_mentions": allowed_users(&[]),
    })
}

fn allowed_users(users: &[UserId]) -> Value {
    json!({
        "parse": [],
        "users": users.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
    })
}

fn text(content: impl Into<String>) -> Value {
    json!({ "type": 10, "content": content.into() })
}

fn separator(divider: bool) -> Value {
    json!({ "type": 14, "divider": divider, "spacing": 1 })
}

fn gallery(urls: &[&str]) -> Value {
    let items: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "media": { "url": url } }))
        .collect();
    json!({ "type": 12, "items": items })
}

fn container(accent: u32, children: Vec<Value>) -> Value {
    json!({ "type": 17, "accent_color": accent, "components": children })
}

fn row(components: Vec<Value>) -> Value {
    json!({ "type": 1, "components": components })
}

fn button(style: u8, label: &str, emoji: &str, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "emoji": { "name": emoji },
        "custom_id": custom_id,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn random_u64() -> u64 {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // İşletim sistemi rastgeleliği yoksa saat tabanlı tohum yeterli (oyun, güvenlik değil).
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default();
        return splitmix64(nanos);
    }
    u64::from_le_bytes(bytes)
}

/// Kararlı karıştırma fonksiyonu: `DefaultHasher` Rust sürümleri arasında değişebileceği
/// için uyum skoru bunun üzerine kuruludur (skor güncellemelerle değişmesin).
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u64) -> UserId {
        UserId::new(id)
    }

    /// Kart gövdesindeki tüm `Text Display` içeriklerinin toplam uzunluğu.
    fn text_chars(value: &Value) -> usize {
        match value {
            Value::Object(map) => {
                let own = if map.get("type") == Some(&json!(10)) {
                    map["content"].as_str().map_or(0, |s| s.chars().count())
                } else {
                    0
                };
                own + map.values().map(text_chars).sum::<usize>()
            }
            Value::Array(items) => items.iter().map(text_chars).sum(),
            _ => 0,
        }
    }

    fn custom_ids(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(id) = map.get("custom_id").and_then(Value::as_str) {
                    out.push(id.to_string());
                }
                map.values().for_each(|v| custom_ids(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| custom_ids(v, out)),
            _ => {}
        }
    }

    #[test]
    fn compatibility_is_symmetric_and_stable() {
        let (a, b) = (user(111_111_111_111), user(222_222_222_222));
        let ab = compatibility(a, b);
        let ba = compatibility(b, a);
        assert_eq!(ab.score, ba.score);
        assert_eq!(ab.traits, ba.traits);
        assert!(ab.score <= 100);
        assert!(ab.traits.iter().all(|value| *value <= 100));
        // Sabit tohum: skor sürümler arasında değişmemeli.
        assert_eq!(compatibility(a, b).score, ab.score);
    }

    #[test]
    fn self_love_is_perfect() {
        let result = compatibility(user(5), user(5));
        assert_eq!(result.score, 100);
        assert_eq!(result.traits, [100; TRAITS.len()]);
    }

    #[test]
    fn scores_spread_across_the_range() {
        let scores: Vec<u8> = (1..400)
            .map(|i| compatibility(user(i), user(i * 7919 + 13)).score)
            .collect();
        assert!(scores.iter().any(|s| *s < 20));
        assert!(scores.iter().any(|s| *s > 80));
    }

    #[test]
    fn ship_name_merges_halves() {
        assert_eq!(ship_name("Ayşe", "Mehmet"), "Aymet");
        assert_eq!(ship_name("Ali", "Can"), "Alan");
        assert_eq!(ship_name("🌸", "Can"), "Bilinmeyen Ship");
    }

    #[test]
    fn bar_has_fixed_width() {
        assert_eq!(bar(0, 10), "▱▱▱▱▱▱▱▱▱▱");
        assert_eq!(bar(100, 10), "▰▰▰▰▰▰▰▰▰▰");
        assert_eq!(bar(50, 8).chars().count(), 8);
        assert_eq!(bar(250, 4), "▰▰▰▰");
    }

    #[test]
    fn moves_form_a_cycle() {
        for mv in Move::ALL {
            assert!(!mv.beats(mv));
            assert_eq!(Move::from_code(mv.code()), Some(mv));
            let beaten = Move::ALL.iter().filter(|other| mv.beats(**other)).count();
            assert_eq!(beaten, 1, "{mv:?} tam olarak bir hamleyi yenmeli");
        }
    }

    #[test]
    fn duel_is_best_of_three() {
        let mut duel = Duel::new(user(1), user(2), false);
        assert_eq!(duel.play(0, Move::Rock), PlayOutcome::Waiting);
        assert_eq!(duel.play(0, Move::Paper), PlayOutcome::AlreadyPicked);
        assert_eq!(duel.play(1, Move::Scissors), PlayOutcome::RoundOver);
        assert_eq!(duel.wins, [1, 0]);
        assert!(duel.result.is_none());

        // Beraberlik skoru değiştirmez.
        duel.play(0, Move::Paper);
        duel.play(1, Move::Paper);
        assert_eq!(duel.wins, [1, 0]);

        duel.play(1, Move::Rock);
        duel.play(0, Move::Paper);
        assert_eq!(
            duel.result,
            Some(DuelResult::Winner {
                index: 0,
                forfeit: false
            })
        );
        assert_eq!(duel.rounds.len(), 3);
    }

    #[test]
    fn endless_draws_end_the_duel() {
        let mut duel = Duel::new(user(1), user(2), false);
        for _ in 0..MAX_DUEL_ROUNDS {
            duel.play(0, Move::Rock);
            duel.play(1, Move::Rock);
        }
        assert_eq!(duel.result, Some(DuelResult::Draw));
    }

    #[test]
    fn bot_rival_always_has_a_pick_ready() {
        let mut duel = Duel::new(user(1), user(2), true);
        assert!(duel.picks[1].is_some());
        assert_eq!(duel.play(0, Move::Rock), PlayOutcome::RoundOver);
        if duel.result.is_none() {
            assert!(duel.picks[1].is_some(), "bot sonraki tur için seçmeli");
        }
    }

    #[test]
    fn forfeit_hands_the_win_to_the_rival() {
        let mut duel = Duel::new(user(1), user(2), false);
        duel.forfeit(1);
        assert_eq!(
            duel.result,
            Some(DuelResult::Winner {
                index: 0,
                forfeit: true
            })
        );
    }

    #[test]
    fn guessing_narrows_the_range() {
        let mut game = Guessing::new(user(1), 100, 42);
        assert_eq!(game.guess(user(2), 0), GuessOutcome::OutOfRange);
        assert_eq!(game.guess(user(2), 101), GuessOutcome::OutOfRange);
        assert_eq!(game.attempts, 0);

        assert_eq!(game.guess(user(2), 50), GuessOutcome::Wrong(Hint::Lower));
        assert_eq!((game.low, game.high), (1, 49));
        assert_eq!(game.guess(user(3), 30), GuessOutcome::Wrong(Hint::Higher));
        assert_eq!((game.low, game.high), (31, 49));
        // Aralık dışı (ama 1..=max içinde) tahmin aralığı genişletmez.
        game.guess(user(3), 90);
        assert_eq!((game.low, game.high), (31, 49));

        assert_eq!(game.guess(user(4), 42), GuessOutcome::Correct);
        assert_eq!(game.winner, Some(user(4)));
        assert_eq!(game.attempts, 4);
        assert!(game.finished());
    }

    #[test]
    fn heat_reflects_distance() {
        let game = Guessing::new(user(1), 100, 50);
        assert_eq!(game.heat(51), "🔥 kaynıyor");
        assert_eq!(game.heat(1), "🥶 buz gibi");
    }

    #[test]
    fn range_bar_marks_the_open_window() {
        assert_eq!(range_bar(1, 100, 100, 10), "▓▓▓▓▓▓▓▓▓▓");
        assert_eq!(range_bar(31, 49, 100, 10), "░░░▓▓░░░░░");
        assert_eq!(range_bar(42, 42, 100, 10), "░░░░▓░░░░░");
    }

    #[test]
    fn game_store_expires_and_removes() {
        let store: GameStore<Guessing> = GameStore::new();
        store.insert(1, Guessing::new(user(1), 100, 5));
        assert_eq!(store.with(1, |game| game.secret), Some(5));
        store.remove(1);
        assert!(store.with(1, |game| game.secret).is_none());

        let mut expired = Guessing::new(user(1), 100, 5);
        expired.expires_at = 0;
        store.insert(2, expired);
        assert!(store.with(2, |_| ()).is_none(), "süresi dolan oyun düşmeli");
    }

    #[test]
    fn game_store_is_bounded() {
        let store: GameStore<Guessing> = GameStore::new();
        for id in 0..(MAX_GAMES as u64 + 10) {
            store.insert(id, Guessing::new(user(1), 100, 5));
        }
        assert_eq!(store.lock().len(), MAX_GAMES);
    }

    #[test]
    fn cards_fit_discord_limits() {
        let long_name = "Ç".repeat(32);
        let profile = |id| Profile {
            id: user(id),
            name: long_name.clone(),
            avatar: "https://cdn.discordapp.com/embed/avatars/0.png".to_string(),
            bot: true,
        };
        let mut duel = Duel::new(user(u64::MAX / 2), user(u64::MAX / 3), false);
        for _ in 0..MAX_DUEL_ROUNDS {
            duel.play(0, Move::Rock);
            duel.play(1, Move::Rock);
        }
        let mut guessing = Guessing::new(user(u64::MAX / 2), 1000, 999);
        for value in 1..30 {
            guessing.guess(user(u64::MAX / 3), value);
        }

        let cards = [
            compat_card(&profile(1), &profile(2)),
            compat_card(&profile(1), &profile(1)),
            duel_card(u64::MAX, &duel),
            duel_card(u64::MAX, &Duel::new(user(1), user(2), false)),
            guess_card(u64::MAX, &guessing),
            eight_ball_response(user(u64::MAX), &"?".repeat(500))["data"].clone(),
        ];
        for card in &cards {
            assert!(text_chars(card) <= 4000, "metin sınırı aşıldı: {card}");
            let mut ids = Vec::new();
            custom_ids(card, &mut ids);
            for id in ids {
                assert!(id.len() <= 100, "custom_id çok uzun: {id}");
                assert!(id.starts_with(CUSTOM_ID_PREFIX));
            }
            let children = card["components"][0]["components"].as_array().unwrap();
            assert!(children.len() <= 10, "container en fazla 10 bileşen alır");
        }
    }

    #[test]
    fn guess_modal_fits_discord_limits() {
        let id = 77;
        GUESSES.insert(id, Guessing::new(user(1), 1000, 3));
        let modal = guess_modal(&id.to_string());
        assert_eq!(modal["type"], RESPONSE_MODAL);
        assert!(modal["data"]["title"].as_str().unwrap().chars().count() <= 45);
        let label = modal["data"]["components"][0]["components"][0]["label"]
            .as_str()
            .unwrap();
        assert!(label.chars().count() <= 45);
        GUESSES.remove(id);

        // Bitmiş/bilinmeyen oyun modal açmaz, gizli uyarı döner.
        assert_eq!(guess_modal("12345")["data"]["flags"], EPHEMERAL_FLAG);
    }

    #[test]
    fn command_definitions_match_discord_schema() {
        let definitions = definitions();
        assert_eq!(definitions.len(), 5);
        for definition in &definitions {
            let json = serde_json::to_value(definition).unwrap();
            let name = json["name"].as_str().unwrap();
            assert!(name.chars().count() <= 32);
            assert_eq!(json["contexts"], json!([0, 1, 2]), "{name}");
            assert_eq!(json["integration_types"], json!([0, 1]), "{name}");
            if json["type"] == 2 {
                assert_eq!(json["description"], "");
            } else {
                assert_eq!(name, name.to_lowercase());
                assert!(!name.contains(' '));
                let description = json["description"].as_str().unwrap();
                assert!(!description.is_empty() && description.chars().count() <= 100);
            }
        }
    }
}
