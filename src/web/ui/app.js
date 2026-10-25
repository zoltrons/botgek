/* ============================================================================
   BOTGEK kontrol paneli · istemci mantığı

   Sunucu sözleşmesi (Rust tarafı, src/web/mod.rs):
     GET  /api/info           → { version, full_version, display_version, is_release,
                                  is_dirty, git_sha, git_tag, commits_since_tag,
                                  build_date, target, port }
     GET  /api/settings       → AppSettings (snake_case)
     POST /api/settings       → AppSettings gövdesi
     GET  /api/discord/roles  → [{ guild_id, guild_name, roles[] }]
     GET  /api/discord/channels → [{ guild_id, guild_name, channels[] }]
     GET  /api/health         → "OK"

   Kapsam (scope): 'global' genel varsayılanlardır; bir sunucu kimliği seçilirse o
   sunucuya özel ayarlar düzenlenir. Sunucu ve kanal ayarı **bölüm bazlı sparse**'tır:
   yalnızca kullanıcının bilerek değiştirdiği bölümler saklanır. Kanal çözümlemesi
   global → sunucu → (yalnızca thread için) parent → kanal sırasındadır; kategori mirası
   yoktur.

   Kural: panelde satır içi olay yöneticisi (onclick=) ve satır içi stil yoktur;
   CSP script-src 'self' bu yüzden gevşetilmedi.
   ========================================================================= */
'use strict';

async function apiFetch(path, options = {}) {
    const response = await fetch(path, {
        ...options, credentials: 'same-origin',
        headers: { ...options.headers, 'X-Botgek-Request': '1' }
    });
    if (response.status === 401) {
        window.location.replace('/login');
        throw new Error('Oturum süresi doldu');
    }
    return response;
}

/* ── sabitler: Rust varsayılanlarıyla birebir aynı ───────────────────────── */
const MODULE_KEYS = ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'spotify', 'twitch', 'pixiv', 'tumblr', 'facebook', 'youtube', 'letterboxd', 'ai_chat'];
const MODULE_DEFAULTS = { twitter: true, tiktok: true, instagram: true, reddit: true, bluesky: true, spotify: true, twitch: true, pixiv: true, tumblr: true, facebook: true, youtube: true, letterboxd: true, ai_chat: true };
const MODULE_LABELS = {
    twitter: 'Twitter / X', tiktok: 'TikTok', instagram: 'Instagram', reddit: 'Reddit',
    bluesky: 'Bluesky', spotify: 'Spotify', twitch: 'Twitch', pixiv: 'Pixiv', tumblr: 'Tumblr',
    facebook: 'Facebook', youtube: 'YouTube Posts', letterboxd: 'Letterboxd', ai_chat: 'AI sohbet'
};
const AI_DEFAULTS = { model: 'google/gemma-4-31b-it:free', system_prompt: '' };

/* Sunucuya özel ayar **bölümleri** (sparse override). Bir bölüm sabitlendiğinde bütün
   olarak geneli ezer; sabitlenmeyen bölüm genelden miras alır (alan bazlı kısmi miras yok). */
const BLOCK_KEYS = ['modules', 'role_filters', 'ai', 'engine'];
const BLOCK_LABELS = { modules: 'Modüller', role_filters: 'Rol filtresi', ai: 'AI persona', engine: 'Dönüştürme motoru' };

/* Bütün platformlar native karttır: motor modu, yedek alan adı ve proxy yedeği yoktur. */
const ENGINE_PLATFORMS = [
    { key: 'twitter', label: 'Twitter / X' },
    { key: 'tiktok', label: 'TikTok' },
    { key: 'instagram', label: 'Instagram' },
    { key: 'reddit', label: 'Reddit' },
    { key: 'bluesky', label: 'Bluesky' },
    { key: 'spotify', label: 'Spotify' },
    { key: 'twitch', label: 'Twitch' },
    { key: 'pixiv', label: 'Pixiv' },
    { key: 'tumblr', label: 'Tumblr' },
    { key: 'facebook', label: 'Facebook' },
    { key: 'youtube', label: 'YouTube Posts' },
    { key: 'letterboxd', label: 'Letterboxd' }
];

const ENGINE_DEFAULTS = {
    twitter_source: 'syndication',
    max_images: 4,
    max_video_mb: 8,
    show_metrics: true,
    show_timestamp: true,
    show_community_note: true,
    show_quote: true,
    auto_translate: true,
    translation_target: 'tr',
    impersonate_skip_replies: true,
    impersonate_user: true,
    respond_to_bots: false,
    sensitive_mode: 'hide_always',
    emojis: {
        verified: '<:verified:1553709836859084841>',
        replies: '<:comment:1553712370302451782>',
        reposts: '<:rt:1553712344939495424>',
        likes: '<:like:1553712395057365102>',
        views: '<:metric:1553712416242798712>',
        translation: '<:translation:1476614397249061056>',
        twitter: '<:twitter:1476642012072920355>',
        instagram: '<:instagram:1558099674731122758>',
    reddit: '<:reddit:1558112156195495956>',
        quote: '<:qutoe:1553728214998847569>'
    }
};

/* Emoji alanlarının okunabilir adları (hem ipucu hem değişiklik özeti için). */
const EMOJI_FIELDS = [
    { key: 'verified', label: 'doğrulama işareti' },
    { key: 'replies', label: 'yanıt' },
    { key: 'reposts', label: 'repost' },
    { key: 'likes', label: 'beğeni' },
    { key: 'views', label: 'görüntülenme' },
    { key: 'translation', label: 'çeviri notu' },
    { key: 'twitter', label: 'X logosu' },
    { key: 'instagram', label: 'Instagram logosu' },
    { key: 'reddit', label: 'Reddit logosu' },
    { key: 'quote', label: 'alıntı' }
];

/* Değişiklik özetinde yol yerine insan dili göstermek için sözlük. */
const DIFF_LABELS = {
    'ai.model': 'AI modeli',
    'ai.system_prompt': 'AI kişiliği',
    'engine.twitter_source': 'X veri kaynağı',
    'engine.max_images': 'Maks. görsel sayısı',
    'engine.max_video_mb': 'Maks. video boyutu',
    'engine.show_metrics': 'Etkileşim metrikleri',
    'engine.show_timestamp': 'Gönderi tarihi',
    'engine.show_community_note': 'Topluluk notu',
    'engine.show_quote': 'Alıntılanan gönderi',
    'engine.auto_translate': 'Otomatik çeviri',
    'engine.impersonate_user': 'Taklit',
    'engine.impersonate_skip_replies': 'Yanıtlarda yanıtı koru',
    'engine.respond_to_bots': 'Bot mesajlarına yanıt',
    'engine.sensitive_mode': 'Hassas içerik modu'
};

/* ── durum ───────────────────────────────────────────────────────────────── */
let BOTGEK_INFO = {
    version: '', full_version: '', display_version: '',
    is_release: false, is_dirty: false, git_sha: '', git_tag: null,
    commits_since_tag: null, build_date: '', target: '',
    port: null
};
let appSettings = {
    modules: Object.assign({}, MODULE_DEFAULTS),
    role_filters: {},
    ai: Object.assign({}, AI_DEFAULTS),
    engine: JSON.parse(JSON.stringify(ENGINE_DEFAULTS)),
    guilds: {}
};
let activeServer = 'global';
let activeChannel = null;
/* İstemci tarafı **sparse ayna**: sunucu kimliği → sabitlenmiş bölümler
   (`{ engine: true, modules: true }`). Sunucuya gönderilmez; kaydederken
   `buildGuildsPayload` gövdeyi bu aynadan üretir. */
let rawOverrides = {};
/* Sunucudan gelen sparse gövdenin kanal kısmı; API çevrimdışı olsa da kayıtlı
   kanalların ve bilinmeyen kanal kimliklerinin kaybolmaması için tutulur. */
let rawGuildPayload = {};
let channelConfigs = {};
let currentModule = 'twitter';
let rawGuildsList = [];
let rawChannelsList = [];
let channelsLoaded = false;
let channelFetchError = false;
let discordRoles = [];
let rolesLoaded = false;
let dirty = false;
let savedSnapshot = '';
let pickerCursor = -1;
let modalAction = null;
/* Onay penceresi açılırken odaktaki öğe: kapanınca odak buraya döner. */
let modalReturnFocus = null;
/* "Kaydedildi" onayının geri alma zamanlayıcısı (yerinde onay, bildirim değil). */
let saveConfirmTimer = null;
/* Sunucudaki ayarların yüklendiği andaki revizyonu (`ETag`): kayıt `If-Match` ile gönderir, başka sekme arada kaydettiyse sunucu 412 döner. */
let settingsRevision = null;
let toastTimer = null;
let editRevision = 0;

MODULE_KEYS.forEach(m => { appSettings.role_filters[m] = { mode: 'disabled', roles: [] }; });

/* ── kısa yardımcılar ────────────────────────────────────────────────────── */
const $ = (id) => document.getElementById(id);
const qs = (selector) => document.querySelector(selector);
const qsa = (selector) => Array.prototype.slice.call(document.querySelectorAll(selector));

function prefersReducedMotion() {
    return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
}

function clampInt(value, fallback, min, max) {
    const n = parseInt(value, 10);
    if (Number.isNaN(n)) return fallback;
    const lo = parseInt(min, 10);
    const hi = parseInt(max, 10);
    return Math.min(Number.isNaN(hi) ? n : hi, Math.max(Number.isNaN(lo) ? n : lo, n));
}

function scrollToSection(id) {
    const el = $(id);
    if (!el) return;
    el.scrollIntoView({ behavior: prefersReducedMotion() ? 'auto' : 'smooth', block: 'start' });
}

/* ══ AYAR NESNESİ ═══════════════════════════════════════════════════════════ */

/* Eksik alanları (eski kayıtlar, elle düzenlenmiş settings.json, kısmi sunucu
   ayarları) varsayılanlarla doldurur; eksik alan erişimi paneli kırıyordu. */
function ensureConfigShape(cfg) {
    cfg.modules = cfg.modules || {};
    MODULE_KEYS.forEach(key => {
        if (cfg.modules[key] === undefined) cfg.modules[key] = MODULE_DEFAULTS[key];
    });
    cfg.role_filters = cfg.role_filters || {};
    MODULE_KEYS.forEach(m => {
        const filter = cfg.role_filters[m];
        if (!filter) cfg.role_filters[m] = { mode: 'disabled', roles: [] };
        else {
            if (!filter.mode) filter.mode = 'disabled';
            if (!Array.isArray(filter.roles)) filter.roles = [];
        }
    });
    cfg.ai = cfg.ai || {};
    Object.keys(AI_DEFAULTS).forEach(key => {
        if (cfg.ai[key] === undefined) cfg.ai[key] = AI_DEFAULTS[key];
    });
    getEngineConfig(cfg);
    return cfg;
}

/* Motor ayarlarını (eski `components_v2` alanı dahil) güncel şemaya taşır. */
function getEngineConfig(cfg) {
    cfg.engine = cfg.engine || {};
    const engine = cfg.engine;
    // Kaldırılan proxy alanları eski kayıtlarda kalmış olabilir; sunucuya geri gönderilmez.
    delete engine.modes;
    delete engine.domains;
    delete engine.fallback_to_proxy;
    if (!engine.emojis) engine.emojis = Object.assign({}, ENGINE_DEFAULTS.emojis);
    EMOJI_FIELDS.forEach(field => {
        if (engine.emojis[field.key] === undefined || engine.emojis[field.key] === null) {
            engine.emojis[field.key] = ENGINE_DEFAULTS.emojis[field.key];
        }
    });
    ['twitter_source', 'max_images', 'max_video_mb', 'show_metrics',
      'show_timestamp', 'show_community_note', 'show_quote', 'auto_translate',
      'impersonate_user', 'impersonate_skip_replies', 'respond_to_bots', 'sensitive_mode'].forEach(field => {
         if (engine[field] === undefined || engine[field] === null) engine[field] = ENGINE_DEFAULTS[field];
     });
    if (typeof engine.respond_to_bots !== 'boolean') engine.respond_to_bots = ENGINE_DEFAULTS.respond_to_bots;
    if (engine.sensitive_mode !== 'hide_always' && engine.sensitive_mode !== 'allow_in_nsfw') engine.sensitive_mode = ENGINE_DEFAULTS.sensitive_mode;
    if (typeof engine.translation_target !== 'string' || !/^[a-z][a-z0-9-]{1,15}$/i.test(engine.translation_target)) engine.translation_target = ENGINE_DEFAULTS.translation_target;
    delete engine.components_v2;
    return engine;
}

function clone(value) {
    return JSON.parse(JSON.stringify(value));
}

/* **Tek çözümleme (Rust `resolve_settings` ile aynı semantik):** bölüm override'da varsa
   onu **bütün olarak**, yoksa geneli alır. AI bloğu istisnası: boş model/kişilik alanları
   genelden doldurulur (eski davranış korunur). */
function resolveSettings(global, override) {
    const over = override || {};
    const pickBlock = (key) => clone(over[key] !== undefined && over[key] !== null ? over[key] : global[key]);
    const resolved = {
        modules: pickBlock('modules'),
        role_filters: pickBlock('role_filters'),
        ai: pickBlock('ai'),
        engine: pickBlock('engine')
    };
    if (!resolved.ai.system_prompt || !resolved.ai.system_prompt.trim()) resolved.ai.system_prompt = global.ai.system_prompt;
    if (!resolved.ai.model || !resolved.ai.model.trim()) resolved.ai.model = global.ai.model;
    return resolved;
}

function isThreadChannel(channel) {
    return !!channel && ['public_thread', 'private_thread', 'news_thread'].indexOf(String(channel.kind || '').toLowerCase()) > -1;
}

function normalizeChannel(channel) {
    const item = channel || {};
    return {
        id: String(item.id || ''),
        name: String(item.name || item.id || 'kanal'),
        kind: String(item.kind || 'text').toLowerCase(),
        parent_id: item.parent_id === null || item.parent_id === undefined ? null : String(item.parent_id),
        nsfw: !!item.nsfw
    };
}

/* Kanal çözümlemesi backend ile aynı sırayı kullanır. Kategori (forum dahil)
   ebeveyn değildir; yalnızca Discord thread türleri parent_id üzerinden miras alır. */
function resolveChannelSettings(global, guildOverride, parentOverride, channelOverride, channel) {
    let resolved = ensureConfigShape(resolveSettings(global, guildOverride));
    if (isThreadChannel(channel)) resolved = ensureConfigShape(resolveSettings(resolved, parentOverride));
    return ensureConfigShape(resolveSettings(resolved, channelOverride));
}

function channelById(guildId, channelId) {
    const guild = rawChannelsList.filter(item => String(item.guild_id) === String(guildId))[0];
    if (!guild) return null;
    return (guild.channels || []).map(normalizeChannel).filter(item => item.id === String(channelId))[0] || null;
}

function sparseGuildFor(guildId) {
    const full = appSettings.guilds && appSettings.guilds[guildId];
    const pins = rawOverrides[guildId] || {};
    const sparse = {};
    BLOCK_KEYS.forEach(key => {
        if (pins[key] && full && full[key] !== undefined) {
            sparse[key] = key === 'ai' && rawGuildPayload[guildId] ? rawGuildPayload[guildId].ai : full[key];
        }
    });
    return sparse;
}

function sparseChannelFor(guildId, channelId) {
    const full = channelConfigs[guildId] && channelConfigs[guildId][channelId];
    const pins = rawOverrides[guildId] && rawOverrides[guildId].channels && rawOverrides[guildId].channels[channelId];
    const sparse = {};
    BLOCK_KEYS.forEach(key => {
        if (pins && pins[key] && full && full[key] !== undefined) {
            const raw = rawGuildPayload[guildId] && rawGuildPayload[guildId].channels && rawGuildPayload[guildId].channels[channelId];
            sparse[key] = key === 'ai' && raw ? raw.ai : full[key];
        }
    });
    return sparse;
}

function inheritanceSource(blockKey, guildId, channelId) {
    if (!guildId || !channelId) return 'global';
    const channel = channelById(guildId, channelId);
    const pins = rawOverrides[guildId] || {};
    const channelPins = pins.channels && pins.channels[channelId];
    if (channelPins && channelPins[blockKey]) return 'kanal';
    if (isThreadChannel(channel) && channel.parent_id) {
        const parentPins = pins.channels && pins.channels[channel.parent_id];
        if (parentPins && parentPins[blockKey]) return 'parent kanal';
    }
    return pins[blockKey] ? 'sunucu' : 'global';
}

/* Sunucunun **sabitlenmiş bölümleri** (istemci tarafı sparse ayna). Genel kapsamda null. */
function overrideOf(guildId) {
    return (guildId !== 'global' && rawOverrides[guildId]) ? rawOverrides[guildId] : null;
}

function channelOverrideOf(guildId, channelId) {
    const guild = overrideOf(guildId);
    return guild && guild.channels && guild.channels[channelId] ? guild.channels[channelId] : null;
}

/* Kapsamın çalışma kopyası. Genel kapsamda genelin kendisi; sunucu kapsamında
   genel+override çözümlemesinin **bağımsız** kopyası. Sunucuda kayıt yoksa genel ayarlar
   mutasyona uğramasın diye kopya açılır (miras görünür, kaydedilene kadar yazılmaz). */
function getActiveConfig() {
    if (activeServer === 'global') return ensureConfigShape(appSettings);
    if (!appSettings.guilds) appSettings.guilds = {};
    if (!appSettings.guilds[activeServer]) {
        appSettings.guilds[activeServer] = resolveSettings(ensureConfigShape(appSettings), null);
    }
    const global = ensureConfigShape(appSettings);
    const guildSparse = sparseGuildFor(activeServer);
    const guildConfig = appSettings.guilds[activeServer];
    const guildResolved = resolveSettings(global, guildSparse);
    BLOCK_KEYS.forEach(key => {
        if (!rawOverrides[activeServer] || !rawOverrides[activeServer][key] || key === 'ai') {
            syncConfigObject(guildConfig[key], guildResolved[key]);
        }
    });
    if (!activeChannel) return ensureConfigShape(guildConfig);

    if (!channelConfigs[activeServer]) channelConfigs[activeServer] = {};
    if (!channelConfigs[activeServer][activeChannel]) channelConfigs[activeServer][activeChannel] = ensureConfigShape({});
    const channel = channelById(activeServer, activeChannel);
    const parentSparse = channel && channel.parent_id ? sparseChannelFor(activeServer, channel.parent_id) : {};
    const channelSparse = sparseChannelFor(activeServer, activeChannel);
    const channelConfig = channelConfigs[activeServer][activeChannel];
    const resolved = resolveChannelSettings(global, guildSparse, parentSparse, channelSparse, channel);
    const pins = channelOverrideOf(activeServer, activeChannel) || {};
    BLOCK_KEYS.forEach(key => {
        if (!pins[key] || key === 'ai') syncConfigObject(channelConfig[key], resolved[key]);
    });
    return ensureConfigShape(channelConfig);
}

/* Preserve block/filter/array identities while refreshing inherited values. */
function syncConfigObject(target, source) {
    Object.keys(target).forEach(key => { if (!(key in source)) delete target[key]; });
    Object.keys(source).forEach(key => {
        const value = source[key];
        if (Array.isArray(value) && Array.isArray(target[key])) target[key].splice(0, target[key].length, ...value);
        else if (value && typeof value === 'object' && target[key] && typeof target[key] === 'object') syncConfigObject(target[key], value);
        else target[key] = clone(value);
    });
}

/* Bu bölüm o sunucuda **bilerek** değiştirildi mi? */
function isBlockOverridden(blockKey) {
    if (activeServer === 'global') return false;
    if (activeChannel) {
        const channel = channelOverrideOf(activeServer, activeChannel);
        return !!(channel && channel[blockKey]);
    }
    const override = overrideOf(activeServer);
    return !!(override && override[blockKey]);
}

/* En az bir bölümü sabitlenmiş sunucu "özel" sayılır (kaydı varsa değil). */
function isServerCustomized(guildId) {
    const override = overrideOf(guildId);
    return !!override && (BLOCK_KEYS.some(key => !!override[key]) || !!(override.channels && Object.keys(override.channels).length));
}

/* İlk düzenlemede o bölümü sabitler (miras → özel). Değer çalışma kopyasından okunur ve
   blok **tam** yazılır (`GuildOverride` sözleşmesi: eksik alt anahtar kod varsayılanına
   düşer, genelin o anki değerine değil).

   Sıra kritiktir: **önce sabitle, sonra oku.** `getActiveConfig()` sabitlenmemiş bölümleri
   çözümlenmiş (miras) değerlerle tazeler; sabitlemeden önce çağrılırsa kullanıcının bu
   düzenlemede modele yazdığı değişiklik (ör. rol listesine eklenen rol — DOM'dan yeniden
   okunamaz) silinir ve kaydetme gövdesine hiç girmezdi. */
function ensureBlockOverridden(blockKey) {
    if (activeServer === 'global') return;
    if (isBlockOverridden(blockKey)) return;
    if (!rawGuildPayload[activeServer]) rawGuildPayload[activeServer] = { channels: {} };
    if (!rawGuildPayload[activeServer].channels) rawGuildPayload[activeServer].channels = {};
    if (!rawOverrides[activeServer]) rawOverrides[activeServer] = {};
    if (activeChannel) {
        if (!rawOverrides[activeServer].channels) rawOverrides[activeServer].channels = {};
        if (!rawOverrides[activeServer].channels[activeChannel]) rawOverrides[activeServer].channels[activeChannel] = {};
        rawOverrides[activeServer].channels[activeChannel][blockKey] = true;
        if (!rawGuildPayload[activeServer].channels[activeChannel]) rawGuildPayload[activeServer].channels[activeChannel] = {};
    } else if (!rawOverrides[activeServer][blockKey]) {
        rawOverrides[activeServer][blockKey] = true;
    }
    // Yalnızca AI bloğu başlangıç değerini çalışma kopyasından alır; diğer bölümler
    // `commitDomToActiveConfig` ile DOM'dan yeniden okunur (tazeleme gerekmez).
    if (blockKey !== 'ai') return;
    const cfg = getActiveConfig();
    if (activeChannel) rawGuildPayload[activeServer].channels[activeChannel].ai = clone(cfg.ai);
    else rawGuildPayload[activeServer].ai = clone(cfg.ai);
}

/* Bir bölümü genele sıfırlar: değerler genelden tazelenir, sabitleme kaldırılır. */
function resetBlockToGlobal(blockKey) {
    if (activeServer === 'global') return;
    const cfg = getActiveConfig();
    if (activeChannel) {
        const channel = channelById(activeServer, activeChannel);
        const guildSparse = sparseGuildFor(activeServer);
        const parentSparse = channel && channel.parent_id ? sparseChannelFor(activeServer, channel.parent_id) : {};
        cfg[blockKey] = clone(resolveChannelSettings(ensureConfigShape(appSettings), guildSparse, parentSparse, {}, channel)[blockKey]);
        const channelPins = rawOverrides[activeServer] && rawOverrides[activeServer].channels && rawOverrides[activeServer].channels[activeChannel];
        if (channelPins) {
            delete channelPins[blockKey];
            if (!Object.keys(channelPins).length) delete rawOverrides[activeServer].channels[activeChannel];
        }
        if (rawOverrides[activeServer] && rawOverrides[activeServer].channels && !Object.keys(rawOverrides[activeServer].channels).length) delete rawOverrides[activeServer].channels;
    } else {
        cfg[blockKey] = clone(ensureConfigShape(appSettings)[blockKey]);
        if (rawOverrides[activeServer]) {
            delete rawOverrides[activeServer][blockKey];
            if (!Object.keys(rawOverrides[activeServer]).some(key => key === 'channels' || rawOverrides[activeServer][key])) delete rawOverrides[activeServer];
        }
    }
    markDirty();
    refreshScope();
    renderUI();
    showToast(BLOCK_LABELS[blockKey] + ' · genele sıfırlandı');
}

function getCurFilter() {
    const cfg = getActiveConfig();
    if (!cfg.role_filters) cfg.role_filters = {};
    if (!cfg.role_filters[currentModule]) cfg.role_filters[currentModule] = { mode: 'disabled', roles: [] };
    return cfg.role_filters[currentModule];
}

/* Sunucudan gelen gövdeyi çalışma modeline çevirir: genel ayarlar tam şemaya,
   sunucu kayıtları **sparse ayna + çözümlenmiş tam kopya** olarak açılır. */
function normalizeFromServer(data) {
    const global = ensureConfigShape({
        modules: Object.assign({}, MODULE_DEFAULTS, (data && data.modules) || {}),
        role_filters: (data && data.role_filters) || {},
        ai: Object.assign({}, AI_DEFAULTS, (data && data.ai) || {}),
        engine: (data && data.engine) || {}
    });
    MODULE_KEYS.forEach(m => {
        if (!global.role_filters[m]) global.role_filters[m] = { mode: 'disabled', roles: [] };
    });

    rawOverrides = {};
    rawGuildPayload = {};
    channelConfigs = {};
    const guilds = {};
    const rawGuilds = (data && data.guilds) || {};
    Object.keys(rawGuilds).forEach(guildId => {
        const sparse = rawGuilds[guildId] || {};
        const pinned = {};
        BLOCK_KEYS.forEach(block => {
            if (sparse[block] !== undefined && sparse[block] !== null) pinned[block] = true;
        });
        const channels = sparse.channels && typeof sparse.channels === 'object' ? sparse.channels : {};
        const channelPins = {};
        Object.keys(channels).forEach(channelId => {
            const channel = channels[channelId] || {};
            const cp = {};
            BLOCK_KEYS.forEach(block => {
                if (channel[block] !== undefined && channel[block] !== null) cp[block] = true;
            });
            // Empty channel maps are retained: they are meaningful saved/offline scopes.
            channelPins[String(channelId)] = cp;
        });
        // Channel-only guild records must remain in the model and payload.
        if (Object.keys(pinned).length || Object.keys(channelPins).length || sparse.channels !== undefined) {
            rawOverrides[guildId] = pinned;
            if (Object.keys(channelPins).length || sparse.channels !== undefined) rawOverrides[guildId].channels = channelPins;
            rawGuildPayload[guildId] = { ai: clone(sparse.ai || {}), channels: clone(channels) };
            guilds[guildId] = ensureConfigShape(resolveSettings(global, sparse));
            channelConfigs[guildId] = {};
            Object.keys(channels).forEach(channelId => {
                channelConfigs[guildId][String(channelId)] = resolveChannelSettings(global, sparse, {}, channels[channelId], null);
            });
            guilds[guildId].channels = channelConfigs[guildId];
            return;
        }
        // No pinned block and no channel map means there is no sparse guild record.
        return;
    });

    return {
        modules: global.modules,
        role_filters: global.role_filters,
        ai: global.ai,
        engine: global.engine,
        guilds
    };
}

/* Sunucu kayıtlarının **sparse** gövdesi: yalnızca sabitlenmiş bölümler yazılır. */
function buildGuildsPayload() {
    const out = {};
    const guildIds = {};
    Object.keys(appSettings.guilds || {}).forEach(id => { guildIds[id] = true; });
    Object.keys(rawGuildPayload || {}).forEach(id => { guildIds[id] = true; });
    Object.keys(rawOverrides || {}).forEach(id => { guildIds[id] = true; });
    Object.keys(guildIds).forEach(guildId => {
        const pinned = rawOverrides[guildId];
        const full = appSettings.guilds[guildId] || resolveSettings(ensureConfigShape(appSettings), null);
        const sparse = {};
        BLOCK_KEYS.forEach(block => {
            if (pinned && pinned[block]) sparse[block] = block === 'ai' ? rawGuildPayload[guildId].ai : full[block];
        });
        const originalChannels = rawGuildPayload[guildId] && rawGuildPayload[guildId].channels || {};
        const channelPins = pinned && pinned.channels || {};
        const channelIds = {};
        Object.keys(originalChannels).forEach(id => { channelIds[id] = true; });
        Object.keys(channelPins).forEach(id => { channelIds[id] = true; });
        if (channelConfigs[guildId]) Object.keys(channelConfigs[guildId]).forEach(id => { channelIds[id] = true; });
        const channels = {};
        Object.keys(channelIds).forEach(channelId => {
            const cpins = channelPins[channelId] || {};
            const fullChannel = channelConfigs[guildId] && channelConfigs[guildId][channelId];
            const original = originalChannels[channelId] || {};
            const channelSparse = {};
            BLOCK_KEYS.forEach(block => {
                if (cpins[block] && fullChannel && fullChannel[block] !== undefined) channelSparse[block] = block === 'ai' ? original.ai : fullChannel[block];
                else if (cpins[block] && original[block] !== undefined) channelSparse[block] = original[block];
            });
            // Keep an offline/unknown channel key and all already pinned blocks intact.
            if (Object.keys(channelSparse).length) channels[channelId] = channelSparse;
        });
        if (Object.keys(channels).length) sparse.channels = channels;
        if (Object.keys(sparse).length) out[guildId] = sparse;
    });
    return out;
}

/* Panelin sunucuya göndereceği gövde — kaydedilen tek gerçek; dirty karşılaştırması da
   bunun üzerinden yapılır (böylece bir bölümü sabitlemek gerçek bir değişiklik sayılır). */
function buildPayload() {
    return {
        modules: appSettings.modules,
        role_filters: appSettings.role_filters,
        ai: appSettings.ai,
        engine: appSettings.engine,
        guilds: buildGuildsPayload()
    };
}

/* ══ DEĞİŞİKLİK TAKİBİ ══════════════════════════════════════════════════════
   Kaydedilmemiş değişiklik hem sekme kapanışında uyarı, hem de "N alan
   değişti" özeti için kullanılır. Sadece okuma amaçlı bir yaprak karşılaştırması. */

function flatten(value, prefix, out) {
    if (value === null || typeof value !== 'object') {
        out[prefix] = JSON.stringify(value);
        return out;
    }
    if (Array.isArray(value)) {
        out[prefix] = JSON.stringify(value);
        return out;
    }
    Object.keys(value).forEach(key => flatten(value[key], prefix ? prefix + '.' + key : key, out));
    return out;
}

function changedFields() {
    if (!savedSnapshot) return [];
    const before = flatten(JSON.parse(savedSnapshot), '', {});
    const now = flatten(buildPayload(), '', {});
    const keys = {};
    Object.keys(before).forEach(k => { keys[k] = true; });
    Object.keys(now).forEach(k => { keys[k] = true; });
    return Object.keys(keys).filter(k => before[k] !== now[k]);
}

function labelFor(path) {
    const parts = path.split('.');
    if (parts[0] === 'guilds' && parts.length > 2) return labelFor(parts.slice(2).join('.'));
    if (DIFF_LABELS[path]) return DIFF_LABELS[path];
    const last = parts[parts.length - 1];
    if (parts[0] === 'modules') return 'Modül: ' + (MODULE_LABELS[last] || last);
    if (parts[0] === 'role_filters') {
        const mod = MODULE_LABELS[parts[1]] || parts[1];
        if (last === 'mode') return 'Filtre modu: ' + mod;
        if (last === 'roles') return 'Rol listesi: ' + mod;
    }
    if (parts.indexOf('emojis') > -1) {
        const field = EMOJI_FIELDS.filter(f => f.key === last)[0];
        if (field) return 'Emoji: ' + field.label;
    }
    return path;
}

function markDirty() {
    dirty = true;
    editRevision++;
}

/* Kayıt durumu: ray göstergesi + dar ekranda çıkan kaydetme çubuğu. */
function updateDirtyUI() {
    const changes = dirty ? changedFields() : [];
    const line = $('dirty-indicator');
    const text = $('dirty-text');
    if (line) line.classList.toggle('is-dirty', dirty);
    if ($('dirty-led')) $('dirty-led').dataset.state = dirty ? 'pending' : 'ok';
    if (text) text.textContent = dirty ? changes.length + ' alan değişti' : 'kayıtlı';

    const dock = $('dock');
    if (!dock) return;
    if (dirty) {
        const names = changes.map(labelFor);
        const head = names.slice(0, 3).join(' · ');
        const rest = names.length > 3 ? ' +' + (names.length - 3) : '';
        $('dock-text').textContent = 'kaydedilmemiş: ' + (head || 'değişiklik') + rest;
    }
    dock.hidden = !dirty;
}

/* Ayarlar satırındaki anahtarlar, satırdaki başlık ve açıklamayı **işaret eder**: ekran
   okuyucu kısa adı ve nedeni birlikte duyurur. HTML'deki `aria-label` yalnızca yedek kalır
   (JavaScript yoksa ad yine doğru olur); bağlama çalışınca kaldırılır. Id'ler burada
   üretildiği için yeni bir kart eklemek yeterlidir, elle id yazmak gerekmez. */
function wireSwitchLabels() {
    qsa('[data-engine-switch]').forEach(input => {
        const tile = input.closest('.tile');
        if (!tile) return;
        const key = input.dataset.engineSwitch || 'x';
        const title = tile.querySelector('.tile-name');
        const hint = tile.querySelector('.tile-hint');
        if (title) {
            if (!title.id) title.id = 'tile-title-' + key;
            input.setAttribute('aria-labelledby', title.id);
        }
        if (hint) {
            if (!hint.id) hint.id = 'tile-hint-' + key;
            input.setAttribute('aria-describedby', hint.id);
        }
        if (title && hint) input.removeAttribute('aria-label');
    });
}

/* Bir alan değişti: ilgili **bölümü** o sunucuda sabitler (miras → özel), dirty işaretler.
   `blockKey` çağrı yerinde verilir (her bölüm kendi denetimlerini bildirir). */
function onConfigModified(blockKey) {
    const firstPin = blockKey && activeServer !== 'global' && !isBlockOverridden(blockKey);
    if (firstPin) {
        ensureBlockOverridden(blockKey);
    }
    // Commit before any refresh can read the effective configuration again.
    commitDomToActiveConfig(blockKey);
    if (firstPin) refreshScope();
    markDirty();
    updateDirtyUI();
}

/* ══ DOM ↔ AYAR EŞLEMESİ ════════════════════════════════════════════════════
   Tek kaynak data-* öznitelikleri ve kimliklerdir: yeni bir alan eklemek için
   yalnızca index.html'e bir öğe ve burada ilgili listeye bir anahtar yeter. */

function readModulesDom() {
    const modules = {};
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        modules[key] = box ? !!box.checked : !!MODULE_DEFAULTS[key];
    });
    return modules;
}

function writeModulesDom(modules) {
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        if (box) box.checked = !!modules[key];
    });
}

function readAiDom() {
    const model = ($('ai-model-box').value || '').trim();
    return {
        model: model || (activeServer === 'global' ? AI_DEFAULTS.model : ''),
        system_prompt: ($('ai-prompt-box').value || '').trim()
    };
}

function writeAiDom(ai) {
    if ($('ai-model-box')) $('ai-model-box').value = ai.model || '';
    if ($('ai-prompt-box')) $('ai-prompt-box').value = ai.system_prompt || '';
}

function readEngineDom(engine) {
    qsa('[data-engine-switch]').forEach(el => { engine[el.dataset.engineSwitch] = !!el.checked; });
    qsa('[data-engine-range]').forEach(el => {
        const key = el.dataset.engineRange;
        engine[key] = clampInt(el.value, ENGINE_DEFAULTS[key], el.min, el.max);
    });
    qsa('[data-emoji-key]').forEach(el => { engine.emojis[el.dataset.emojiKey] = el.value.trim(); });
    const source = $('engine-twitter-source');
    if (source) engine.twitter_source = source.value;
    const translationTarget = $('engine-translation-target');
    if (translationTarget) engine.translation_target = translationTarget.value.trim().toLowerCase() || ENGINE_DEFAULTS.translation_target;
    const sensitive = $('engine-sensitive-mode');
    if (sensitive) engine.sensitive_mode = sensitive.value === 'allow_in_nsfw' ? 'allow_in_nsfw' : 'hide_always';
}

function writeEngineDom(engine) {
    ENGINE_PLATFORMS.forEach(p => updateEngineNote(p.key));
    qsa('[data-engine-switch]').forEach(el => {
        const value = engine[el.dataset.engineSwitch];
        if (typeof value === 'boolean') el.checked = value;
    });
    qsa('[data-engine-range]').forEach(el => {
        const value = engine[el.dataset.engineRange];
        if (typeof value === 'number') el.value = value;
    });
    qsa('[data-emoji-key]').forEach(el => {
        const key = el.dataset.emojiKey;
        el.value = engine.emojis[key] || '';
        el.placeholder = ENGINE_DEFAULTS.emojis[key] || '';
    });
    const source = $('engine-twitter-source');
    if (source) source.value = engine.twitter_source || 'syndication';
    const translationTarget = $('engine-translation-target');
    if (translationTarget) translationTarget.value = engine.translation_target || ENGINE_DEFAULTS.translation_target;
    const sensitive = $('engine-sensitive-mode');
    if (sensitive) sensitive.value = engine.sensitive_mode === 'allow_in_nsfw' ? 'allow_in_nsfw' : 'hide_always';
}

/* Panelden sunucuya gönderilecek gövdeyi DOM'dan toplar. */
function commitDomToActiveConfig(blockKey) {
    const cfg = getActiveConfig();
    const writable = key => (!blockKey || blockKey === key) && (activeServer === 'global' || isBlockOverridden(key));
    if (writable('modules')) Object.assign(cfg.modules, readModulesDom());
    if (writable('engine')) readEngineDom(cfg.engine);
    if (writable('role_filters')) cfg.role_filters[currentModule].mode = getFilterModeDom();
    // AI input events commit immediately. Scope switches/saves must not turn a
    // displayed inherited value into a pinned value for a saved empty field.
    if (blockKey === 'ai' && writable('ai')) {
        const values = readAiDom();
        if (activeServer === 'global') Object.assign(cfg.ai, values);
        else {
            const raw = activeChannel ? rawGuildPayload[activeServer].channels[activeChannel].ai : rawGuildPayload[activeServer].ai;
            Object.keys(values).forEach(key => {
                if (values[key] !== cfg.ai[key]) raw[key] = values[key];
            });
        }
    }
    return cfg;
}

/* ══ GÖRÜNÜM GÜNCELLEMELERİ ════════════════════════════════════════════════ */

function updateModuleUI() {
    const modules = readModulesDom();
    let active = 0;
    MODULE_KEYS.forEach(key => {
        const on = !!modules[key];
        if (on) active++;
        const label = $('status-' + key);
        if (label) {
            label.textContent = on ? 'Aktif' : 'Devre dışı';
            label.classList.toggle('is-on', on);
        }
    });
    if ($('active-modules-count')) $('active-modules-count').textContent = active + '/' + MODULE_KEYS.length + ' aktif';
    if ($('nav-count-modules')) $('nav-count-modules').textContent = active + '/' + MODULE_KEYS.length;
    const hint = $('module-disabled-hint');
    if (hint) hint.hidden = !!modules[currentModule];
    updateNavCounts();
}

function updateModuleBadges() {
    const cfg = getActiveConfig();
    MODULE_KEYS.forEach(key => {
        const badge = $('badge-' + key);
        if (!badge) return;
        const filter = cfg.role_filters[key] || { mode: 'disabled', roles: [] };
        const count = (filter.roles || []).length;
        if (filter.mode === 'blacklist') {
            badge.textContent = 'kara liste · ' + count;
            badge.dataset.tone = 'blacklist';
        } else if (filter.mode === 'allowlist') {
            badge.textContent = 'beyaz liste · ' + count;
            badge.dataset.tone = 'allowlist';
        } else {
            badge.textContent = 'filtre yok';
            delete badge.dataset.tone;
        }
    });
}

function updateNavCounts() {
    const cfg = getActiveConfig();
    const filtered = MODULE_KEYS.filter(k => {
        const f = cfg.role_filters[k];
        return f && f.mode !== 'disabled';
    }).length;
    if ($('nav-count-roles')) $('nav-count-roles').textContent = filtered + '/' + MODULE_KEYS.length;
    const model = (cfg.ai.model || '').split('/').pop().replace(':free', '');
    const state = cfg.modules.ai_chat ? (model || 'hazır') : 'modül kapalı';
    if ($('nav-count-ai')) $('nav-count-ai').textContent = state;
    if ($('ai-state')) $('ai-state').textContent = state;
}

/* Motor satırı: her platform sabit native karttır. */
function updateEngineNote(key) {
    const note = $('engine-note-' + key);
    if (note) {
        note.textContent = 'native embed';
        note.dataset.on = 'true';
    }
}

function updateEngineSummary() {
    const engine = getEngineConfig(getActiveConfig());
    const nativeCount = ENGINE_PLATFORMS.length;
    const images = $('engine-max-images');
    const video = $('engine-max-video');
    if (images && $('engine-max-images-value')) $('engine-max-images-value').textContent = images.value + ' adet';
    if (video && $('engine-max-video-value')) $('engine-max-video-value').textContent = video.value + ' MB';

    const parts = [nativeCount + '/' + ENGINE_PLATFORMS.length + ' native'];
    parts.push('V2 kart');
    if (engine.impersonate_user) parts.push('taklit');
    if (engine.auto_translate) parts.push('otomatik çeviri');
    if ($('engine-summary')) $('engine-summary').textContent = parts.join(' · ');
    if ($('nav-count-engine')) $('nav-count-engine').textContent = nativeCount + '/' + ENGINE_PLATFORMS.length;
}

/* Modül satırındaki adres: bütün platformlar native karttır. */
function updatePlatformHosts() {
    qsa('[data-host-key]').forEach(el => { el.textContent = 'native embed · bot çözer'; });
}

/* Motor bölümündeki alan bağımlılıkları. "Yanıtlarda yanıtı koru" yalnızca "Kullanıcıyı
   taklit et" açıkken anlamlıdır (`should_impersonate = impersonate_user && !(isReply &&
   impersonate_skip_replies)`); taklit kapalıyken alan devre dışı bırakılır ki kullanıcı
   etkisiz bir ayar değiştirmesin. */
function updateEngineDependencies() {
    const engine = getEngineConfig(getActiveConfig());
    qsa('[data-engine-switch="impersonate_skip_replies"]').forEach(el => {
        el.disabled = !engine.impersonate_user;
        const tile = el.closest('.tile');
        if (tile) tile.classList.toggle('is-disabled', el.disabled);
        el.title = el.disabled ? '"Kullanıcıyı taklit et" kapalı: bu ayar etkisizdir' : '';
    });
}

function updateCharCount() {
    const box = $('ai-prompt-box');
    if (box && $('char-counter')) $('char-counter').textContent = box.value.length + ' / 4000';
}

function updatePresetChips() {
    const model = ($('ai-model-box').value || '').trim();
    qsa('[data-model-preset]').forEach(btn => btn.classList.toggle('is-active', btn.dataset.modelPreset === model));
}

/* ══ ROL FİLTRESİ STÜDYOSU ═════════════════════════════════════════════════ */

function getFilterModeDom() {
    const checked = qs('input[name="filter-mode"]:checked');
    return checked ? checked.value : 'disabled';
}

function updateFilterModeUI(mode) {
    const badge = $('filter-mode-badge');
    if (badge) {
        badge.textContent = mode === 'blacklist' ? 'kara liste' : (mode === 'allowlist' ? 'beyaz liste' : 'devre dışı');
    }
    if ($('role-count')) $('role-count').textContent = (getCurFilter().roles || []).length + ' rol';
}

function selectStudioModule(key) {
    currentModule = key;
    qsa('[data-studio-module]').forEach(btn => {
        const on = btn.dataset.studioModule === key;
        btn.classList.toggle('is-active', on);
        btn.setAttribute('aria-selected', on ? 'true' : 'false');
        btn.tabIndex = on ? 0 : -1;
    });
    const panel = $('studio-panel');
    if (panel) panel.setAttribute('aria-labelledby', 'modtab-' + key);

    const filter = getCurFilter();
    const mode = filter.mode || 'disabled';
    qsa('input[name="filter-mode"]').forEach(radio => { radio.checked = radio.value === mode; });
    updateFilterModeUI(mode);
    updateModuleBadges();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');

    const hint = $('module-disabled-hint');
    if (hint) hint.hidden = !!readModulesDom()[key];
}

function setModelPreset(model) {
    if ($('ai-model-box')) $('ai-model-box').value = model;
    onConfigModified('ai');
    updatePresetChips();
    updateNavCounts();
}

/* ── rol çipleri ─────────────────────────────────────────────────────────── */
function renderRoleChips() {
    const container = $('chips-container');
    if (!container) return;
    container.textContent = '';
    const roles = getCurFilter().roles || [];

    if (!roles.length) {
        const hint = document.createElement('span');
        hint.className = 'chips-empty';
        hint.textContent = 'Bu modül için rol eklenmedi.';
        container.appendChild(hint);
        updateFilterModeUI(getFilterModeDom());
        return;
    }

    roles.forEach((roleId, index) => {
        const matched = discordRoles.filter(r => r.id === roleId)[0];
        const chip = document.createElement('span');
        chip.className = 'chip-role';
        chip.title = matched ? '@' + matched.name + ' · ' + roleId : 'Rol bulunamadı: ' + roleId;

        const dot = document.createElement('span');
        dot.className = 'role-dot';
        if (matched) dot.style.setProperty('--role-color', matched.color);
        chip.appendChild(dot);

        const main = document.createElement('span');
        main.className = 'role-main';
        const name = document.createElement('span');
        name.textContent = matched ? '@' + matched.name : '#' + roleId;
        main.appendChild(name);
        const sub = document.createElement('span');
        sub.className = 'chip-role-id';
        sub.textContent = matched ? roleId : 'sunucuda bulunamadı';
        main.appendChild(sub);
        chip.appendChild(main);

        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'chip-remove';
        remove.textContent = '×';
        remove.setAttribute('aria-label', (matched ? matched.name : roleId) + ' rolünü listeden çıkar');
        remove.addEventListener('click', () => removeRole(index));
        chip.appendChild(remove);

        container.appendChild(chip);
    });
    updateFilterModeUI(getFilterModeDom());
}

function toggleRole(roleId) {
    const roles = getCurFilter().roles;
    const index = roles.indexOf(roleId);
    if (index > -1) roles.splice(index, 1);
    else roles.push(roleId);
    onConfigModified('role_filters');
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

function removeRole(index) {
    getCurFilter().roles.splice(index, 1);
    onConfigModified('role_filters');
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

/* Rol kimliği doğrulaması **sunucu kuralıyla birebir aynı** (`RoleFilterSettings::sanitize`):
   yalnızca rakam, `u64` aralığında ve `> 0`. Değer kanonik ondalık biçime çevrilir; böylece
   panelin gönderdiği kimlik ile sunucunun kaydettiği kimlik aynı olur. Aksi hâlde panel
   geçersiz bir kimliği (ör. 21+ haneli ya da `0`) çip olarak gösterir ama sunucu kaydetme
   sırasında onu sessizce düşürürdü. */
const MAX_ROLE_ID = 18446744073709551615n;
function normalizeRoleId(raw) {
    const text = String(raw === undefined || raw === null ? '' : raw).trim();
    if (!/^\d{1,25}$/.test(text)) return null;
    let value;
    try {
        value = BigInt(text);
    } catch (err) {
        return null;
    }
    if (value <= 0n || value > MAX_ROLE_ID) return null;
    return value.toString();
}

/* Elle girilen kimlikler: virgül/boşlukla ayrılabilir; sunucunun kuralı uygulanır. */
function addRoleFromInput() {
    const box = $('role-input-box');
    if (!box) return;
    const raw = (box.value || '').trim();
    if (!raw) return;
    const ids = raw.split(/[\s,]+/).map(normalizeRoleId).filter(id => id !== null);
    const filter = getCurFilter();
    let added = 0;
    ids.forEach(id => {
        if (filter.roles.indexOf(id) === -1) {
            filter.roles.push(id);
            added++;
        }
    });
    box.value = '';
    if (!added) {
        showToast('eklenecek geçerli rol kimliği bulunamadı', true);
        return;
    }
    onConfigModified('role_filters');
    updateModuleBadges();
    updateNavCounts();
    renderRoleChips();
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
}

/* ── rol seçici ──────────────────────────────────────────────────────────── */
function pickerResults(query) {
    const q = (query || '').toLowerCase().trim();
    return discordRoles.filter(r => !q || r.name.toLowerCase().indexOf(q) > -1 || r.id.indexOf(q) > -1);
}

function pickerMessage(text) {
    const el = document.createElement('p');
    el.className = 'picker-empty';
    el.textContent = text;
    return el;
}

function renderRolePickerList(query) {
    const list = $('role-picker-list');
    if (!list) return;
    list.textContent = '';
    // Önceki turdan kalan imleç adresi temizlenir: boş sonuç listesinde bayat id kalmasın.
    const search = $('role-search-input');
    if (search) search.removeAttribute('aria-activedescendant');
    const results = pickerResults(query);
    if ($('role-picker-count')) $('role-picker-count').textContent = results.length + '/' + discordRoles.length;

    if (!discordRoles.length) {
        list.appendChild(pickerMessage(rolesLoaded ? 'Bu kapsam için rol bulunamadı.' : 'Sunucu rolleri yükleniyor…'));
        return;
    }
    if (!results.length) {
        list.appendChild(pickerMessage('Eşleşen rol yok.'));
        return;
    }

    const selected = getCurFilter().roles || [];
    results.forEach((role, index) => {
        const item = document.createElement('div');
        item.className = 'picker-item';
        item.setAttribute('role', 'option');
        // Liste kutusu seçenekleri klavyeyle `aria-activedescendant` ile adreslenir: arama
        // alanı odakta kalır (tek Tab durağı), ok tuşları imleci gezer.
        item.id = 'role-option-' + role.id;
        item.setAttribute('aria-selected', selected.indexOf(role.id) > -1 ? 'true' : 'false');
        if (selected.indexOf(role.id) > -1) item.classList.add('is-selected');
        if (index === pickerCursor) item.classList.add('is-cursor');
        item.addEventListener('click', () => toggleRole(role.id));

        const dot = document.createElement('span');
        dot.className = 'role-dot';
        dot.style.setProperty('--role-color', role.color);
        item.appendChild(dot);

        const body = document.createElement('span');
        body.className = 'picker-item-body';
        const name = document.createElement('span');
        name.className = 'picker-item-name';
        name.textContent = '@' + role.name;
        body.appendChild(name);
        const id = document.createElement('span');
        id.className = 'picker-item-id';
        id.textContent = role.id;
        body.appendChild(id);
        // Genel kapsamda tüm sunucuların rolleri listelenir: aynı adlı rolleri
        // ayırt edebilmek için rolün hangi sunucuya ait olduğu yazılır.
        if (activeServer === 'global' && role.guild_name) {
            const guild = document.createElement('span');
            guild.className = 'picker-item-guild';
            guild.textContent = role.guild_name;
            body.appendChild(guild);
        }
        item.appendChild(body);

        if (selected.indexOf(role.id) > -1) {
            const check = document.createElement('span');
            check.className = 'picker-check';
            check.textContent = 'seçili';
            item.appendChild(check);
        }
        list.appendChild(item);
    });

    syncPickerActiveDescendant();
}

/* Arama alanındaki `aria-activedescendant`, ok tuşlarıyla gezinilen seçeneği gösterir:
   ekran okuyucu odak taşımadan aktif seçeneği duyurur. */
function syncPickerActiveDescendant() {
    const search = $('role-search-input');
    const list = $('role-picker-list');
    if (!search || !list) return;
    const active = pickerCursor > -1 ? list.children[pickerCursor] : null;
    if (active && active.id) search.setAttribute('aria-activedescendant', active.id);
    else search.removeAttribute('aria-activedescendant');
}

function openRolePicker() {
    const pop = $('role-picker-dropdown');
    if (!pop) return;
    pop.hidden = false;
    $('btn-toggle-picker').setAttribute('aria-expanded', 'true');
    const search = $('role-search-input');
    if (search) {
        search.value = '';
        pickerCursor = -1;
        search.setAttribute('aria-expanded', 'true');
        renderRolePickerList('');
        search.focus();
    }
}

function closeRolePicker() {
    const pop = $('role-picker-dropdown');
    if (!pop || pop.hidden) return;
    pop.hidden = true;
    $('btn-toggle-picker').setAttribute('aria-expanded', 'false');
    const search = $('role-search-input');
    if (search) search.setAttribute('aria-expanded', 'false');
}

function toggleRolePicker() {
    const pop = $('role-picker-dropdown');
    if (pop && pop.hidden) openRolePicker();
    else closeRolePicker();
}

function movePickerCursor(delta) {
    const results = pickerResults($('role-search-input').value);
    if (!results.length) return;
    pickerCursor = (pickerCursor + delta + results.length) % results.length;
    const list = $('role-picker-list');
    Array.prototype.slice.call(list.children).forEach((el, index) => {
        el.classList.toggle('is-cursor', index === pickerCursor);
    });
    const active = list.children[pickerCursor];
    if (active && active.scrollIntoView) active.scrollIntoView({ block: 'nearest' });
    syncPickerActiveDescendant();
}

function onPickerKeydown(e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); movePickerCursor(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); movePickerCursor(-1); }
    else if (e.key === 'Enter') {
        e.preventDefault();
        const results = pickerResults($('role-search-input').value);
        if (pickerCursor > -1 && results[pickerCursor]) toggleRole(results[pickerCursor].id);
        else if (results.length === 1) toggleRole(results[0].id);
    }
}

/* ══ KAPSAM (GENEL / SUNUCU) ═══════════════════════════════════════════════ */

function updateServerSelectDropdown() {
    const select = $('server-select');
    if (!select) return;
    const previous = activeServer;
    select.textContent = '';

    const globalOption = document.createElement('option');
    globalOption.value = 'global';
    globalOption.textContent = 'Genel / varsayılan · tüm sunucular';
    select.appendChild(globalOption);

    const guilds = rawGuildsList.slice();
    rawChannelsList.forEach(guild => {
        if (!guilds.some(existing => String(existing.guild_id) === String(guild.guild_id))) guilds.push(guild);
    });
    guilds.forEach(guild => {
        const option = document.createElement('option');
        option.value = guild.guild_id;
        option.textContent = guild.guild_name + (isServerCustomized(guild.guild_id) ? ' · özel' : '');
        select.appendChild(option);
    });

    // Sunucu artık listede yoksa (bot atıldı vb.) görünmez kapsamda kalmayalım.
    select.value = previous;
    if (select.value !== previous) {
        select.value = 'global';
        activeServer = 'global';
    }
}

function channelsForGuild(guildId) {
    const item = rawChannelsList.filter(guild => String(guild.guild_id) === String(guildId))[0];
    const known = item ? (item.channels || []).map(normalizeChannel) : [];
    const saved = rawGuildPayload[guildId] && rawGuildPayload[guildId].channels || {};
    Object.keys(saved).forEach(id => {
        if (!known.some(channel => channel.id === String(id))) {
            known.push({ id: String(id), name: 'kanal ' + id + ' · kayıtlı', kind: 'text', parent_id: null, nsfw: false, offline: true });
        }
    });
    return known;
}

function updateChannelSelect() {
    const wrap = $('channel-scope-wrap');
    const select = $('channel-select');
    if (!wrap || !select) return;
    const guildScope = activeServer !== 'global';
    wrap.hidden = !guildScope;
    if (!guildScope) return;
    const previous = activeChannel;
    select.textContent = '';
    const inherit = document.createElement('option');
    inherit.value = '';
    inherit.textContent = 'Tüm sunucu (kanal seçilmedi)';
    select.appendChild(inherit);
    channelsForGuild(activeServer).forEach(channel => {
        const option = document.createElement('option');
        option.value = channel.id;
        const kind = channel.kind === 'forum' ? 'forum' : (isThreadChannel(channel) ? 'thread' : channel.kind);
        option.textContent = '#' + channel.name + ' · ' + kind + (channel.offline ? ' · çevrimdışı' : '');
        select.appendChild(option);
    });
    select.value = previous || '';
    if (select.value !== (previous || '')) {
        activeChannel = null;
        select.value = '';
    }
    const status = $('channel-fetch-status');
    if (status) {
        status.textContent = channelFetchError
            ? 'Kanal listesi alınamadı; kayıtlı kanallar korunuyor.'
            : (channelsLoaded ? channelsForGuild(activeServer).length + ' kanal' : 'Kanal listesi yükleniyor…');
        status.dataset.tone = channelFetchError ? 'error' : '';
    }
}

function updateScopeUI() {
    const badge = $('server-custom-badge');
    const button = $('btn-toggle-custom');
    const notice = $('server-inherit-notice');
    const echo = $('scope-echo');
    updateChannelSelect();
    const guild = rawGuildsList.filter(g => g.guild_id === activeServer)[0];
    const pickerLabel = $('picker-scope-label');
    if (pickerLabel) {
        // Genel kapsamda **tüm sunucuların** rolleri listelenir (picker'da sunucu adı da
        // yazılır); etiket buna göre değişir, aksi hâlde genelde "sunucu rolü" yokmuş gibi
        // okunuyordu.
        pickerLabel.textContent = activeServer === 'global' ? 'Tüm sunuculardan rol seç' : 'Sunucudan rol seç';
    }

    if (activeServer === 'global') {
        if (badge) { badge.textContent = 'genel varsayılan'; delete badge.dataset.tone; }
        if (button) button.hidden = true;
        if (notice) notice.hidden = true;
        if (echo) echo.textContent = 'genel varsayılan · tüm sunucular';
        updateBlockBadges();
        return;
    }

    const pinned = overrideOf(activeServer) || {};
    const pinnedCount = BLOCK_KEYS.filter(key => !!pinned[key]).length;
    const channelCount = pinned.channels ? Object.keys(pinned.channels).length : 0;
    const customized = pinnedCount > 0 || channelCount > 0;
    if (badge) {
        badge.textContent = customized ? pinnedCount + '/' + BLOCK_KEYS.length + ' bölüm özel' + (channelCount ? ' · ' + channelCount + ' kanal' : '') : 'miras alıyor';
        badge.dataset.tone = customized ? 'custom' : 'inherit';
    }
    if (button) {
        // Bölüm bazlı sıfırlama bölüm başlıklarındaki "genele sıfırla" ile yapılır; bu düğme
        // yalnızca **tüm** sunucu kaydını silmek için görünür.
        button.hidden = !customized;
        button.textContent = 'Varsayılana sıfırla';
    }
    if (notice) notice.hidden = customized;
    if (echo) {
        echo.textContent = (guild ? guild.guild_name : 'sunucu ' + activeServer) +
            (activeChannel ? ' · kanal #' + activeChannel : (customized ? ' · ' + pinnedCount + '/' + BLOCK_KEYS.length + ' bölüm özel' : ' · genel varsayılanlar'));
    }
    updateBlockBadges();
}

/* Bölüm başlıklarındaki "miras / özel" rozeti ve "genele sıfırla" düğmesi.
   Genel kapsamda bölüm override'ı anlamsızdır; rozet ve düğme gizlenir. */
function updateBlockBadges() {
    const guildScope = activeServer !== 'global';
    qsa('[data-block-badge]').forEach(el => {
        if (!guildScope) {
            el.hidden = true;
            delete el.dataset.tone;
            return;
        }
        const overridden = isBlockOverridden(el.dataset.blockBadge);
        el.hidden = false;
        el.textContent = overridden ? 'özel · ' + (activeChannel ? 'kanal' : 'sunucu') : (activeChannel ? inheritanceSource(el.dataset.blockBadge, activeServer, activeChannel) : 'miras');
        el.dataset.tone = overridden ? 'custom' : 'inherit';
    });
    qsa('[data-block-reset]').forEach(el => {
        el.hidden = !(guildScope && isBlockOverridden(el.dataset.blockReset));
        el.textContent = activeChannel ? 'miras ayarına sıfırla' : 'genele sıfırla';
    });
}

function refreshScope() {
    updateServerSelectDropdown();
    updateScopeUI();
}

function onServerChange() {
    commitDomToActiveConfig();
    const select = $('server-select');
    activeServer = select ? select.value : 'global';
    activeChannel = null;
    refreshScope();
    filterDiscordRolesForActiveServer();
    renderUI();
}

function onChannelChange() {
    commitDomToActiveConfig();
    const select = $('channel-select');
    activeChannel = select && select.value ? String(select.value) : null;
    refreshScope();
    renderUI();
}

function toggleServerCustomSettings() {
    if (activeServer === 'global' || !isServerCustomized(activeServer)) return;
    openModal({
        kicker: 'kapsam',
        title: 'Sunucu ayarlarını sıfırla',
        body: 'Bu sunucuda sabitlenmiş bölümler silinecek ve sunucu genel varsayılanları miras alacak. Kanal bazlı ayarlar korunur. Kaydettiğinizde kalıcı olur.',
        confirmLabel: 'Varsayılana sıfırla',
        onConfirm: () => {
            const old = rawOverrides[activeServer] || {};
            BLOCK_KEYS.forEach(key => delete old[key]);
            if (Object.keys(old).some(key => key === 'channels' || old[key])) rawOverrides[activeServer] = old;
            else delete rawOverrides[activeServer];
            // Kanal blokları sunucu blokları sıfırlanırken aynen korunur.
            if (appSettings.guilds[activeServer]) {
                const fresh = resolveSettings(ensureConfigShape(appSettings), {});
                BLOCK_KEYS.forEach(key => { appSettings.guilds[activeServer][key] = fresh[key]; });
            }
            markDirty();
            refreshScope();
            renderUI();
            showToast('sunucu ayarları varsayılana sıfırlandı');
        }
    });
}

/* ══ AĞ ════════════════════════════════════════════════════════════════════ */

/* Sürüm rozetini `/api/info` verisinden doldurur.
   Yayın sürümünde `v0.2.0`; geliştirme derlemesinde `v0.2.0+5` (etiketten bu yana 5 commit)
   veya `+dev`; kirli ağaçta `*`. Tam künye (sha, tarih, hedef, etiket) `title` içinde. */
function updateVersionBadge() {
    const badge = $('app-version');
    if (!badge) return;
    const semver = BOTGEK_INFO.version || '—';
    const commits = BOTGEK_INFO.commits_since_tag;
    const release = !!BOTGEK_INFO.is_release;
    const suffix = release ? '' : (commits ? '+' + commits : '+dev');
    badge.textContent = 'v' + semver + suffix + (BOTGEK_INFO.is_dirty ? '*' : '');
    badge.dataset.tone = release ? 'release' : 'dev';
    badge.title = [
        BOTGEK_INFO.full_version || 'v' + semver,
        release ? 'yayın sürümü' : (BOTGEK_INFO.is_dirty ? 'geliştirme · kirli ağaç' : 'geliştirme derlemesi'),
        'commit ' + (BOTGEK_INFO.git_sha || 'unknown'),
        BOTGEK_INFO.git_tag
            ? BOTGEK_INFO.git_tag + (commits ? ' +' + commits + ' commit' : ' (tam isabet)')
            : 'etiket yok',
        BOTGEK_INFO.build_date || 'bilinmeyen tarih',
        BOTGEK_INFO.target || 'bilinmeyen hedef'
    ].join(' · ');
}

/* Sürüm, gerçek WebUI portu ve .env kaynaklı yedek alan adları. */
async function fetchInfo() {
    try {
        const res = await apiFetch('/api/info');
        if (res.ok) BOTGEK_INFO = await res.json();
    } catch (err) {
        console.warn('api/info okunamadı', err);
    }

    updateVersionBadge();
    if ($('brand-port') && BOTGEK_INFO.port) $('brand-port').textContent = 'webui · port ' + BOTGEK_INFO.port;
    updatePlatformHosts();
}

async function fetchSettings() {
    try {
        const res = await apiFetch('/api/settings');
        if (!res.ok) throw new Error('HTTP ' + res.status);
        settingsRevision = res.headers.get('ETag');
        appSettings = normalizeFromServer(await res.json());
        savedSnapshot = JSON.stringify(buildPayload());
        dirty = false;
        refreshScope();
        renderUI();
        updateDirtyUI();
    } catch (err) {
        console.error('ayarlar okunamadı', err);
        showToast('ayarlar okunamadı · ' + err.message, true);
    }
}

async function fetchDiscordRoles() {
    try {
        const res = await apiFetch('/api/discord/roles');
        if (!res.ok) throw new Error('HTTP ' + res.status);
        rawGuildsList = await res.json();
        rolesLoaded = true;
        refreshScope();
        filterDiscordRolesForActiveServer();
    } catch (err) {
        // "Yükleniyor…" sonsuza dek kalmasın: seçici boş liste mesajına geçer.
        rolesLoaded = true;
        console.warn('roller okunamadı', err);
        showToast('sunucu rolleri alınamadı · rol kimliği elle girilebilir', true);
    }
}

async function fetchDiscordChannels() {
    try {
        const res = await apiFetch('/api/discord/channels');
        if (!res.ok) throw new Error('HTTP ' + res.status);
        const result = await res.json();
        if (!Array.isArray(result)) throw new Error('geçersiz kanal listesi');
        rawChannelsList = result.map(guild => ({
            guild_id: String(guild.guild_id),
            guild_name: String(guild.guild_name || guild.guild_id),
            channels: Array.isArray(guild.channels) ? guild.channels.map(normalizeChannel).filter(channel => channel.id) : []
        }));
        channelsLoaded = true;
        channelFetchError = false;
        refreshScope();
    } catch (err) {
        // Known saved scopes remain visible and editable after API failure.
        channelFetchError = true;
        updateChannelSelect();
        showToast('kanallar alınamadı · kayıtlı kanal ayarları korunuyor', true);
    }
}

function retryDiscordChannels() {
    fetchDiscordChannels();
}

/* Gerçek erişilebilirlik: başlıktaki lamba tahmine değil /api/health yanıtına bakar. */
async function checkHealth() {
    const led = $('led');
    const status = $('system-status');
    try {
        const res = await apiFetch('/api/ready');
        const readiness = await res.json();
        if (!res.ok) throw new Error(!readiness.discord ? 'Discord bağlantısı hazır değil' : 'Kalıcı veritabanı hazır değil');
        if (led) led.dataset.state = 'live';
        if (status) status.textContent = 'sistem canlı · port ' + (BOTGEK_INFO.port || '—');
        if ($('foot-api')) $('foot-api').textContent = 'api · sağlıklı · port ' + (BOTGEK_INFO.port || '—');
    } catch (err) {
        if (led) led.dataset.state = 'down';
        if (status) status.textContent = err.message || 'sunucu yanıt vermiyor';
        if ($('foot-api')) $('foot-api').textContent = 'api · yanıt yok';
    }
}

/* Birincil düğmenin özgün etiketi bir kez yakalanır: art arda kayıtlar ("Kaydediliyor…" /
   "Kaydedildi") geçici etiketi kalıcı etiket sanmasın. */
let saveButtonLabel = null;
let saveInFlight = false;

function restoreSaveButton(button) {
    if (saveButtonLabel !== null) button.innerHTML = saveButtonLabel;
}

function setSaving(busy) {
    ['btn-save', 'btn-save-dock'].forEach(id => {
        const el = $(id);
        if (el) el.disabled = busy;
    });
    // Kaydetme sürerken bölge "meşgul" işaretlenir: ekran okuyucu bekler ve tekrar denemez.
    const region = $('main');
    if (region) {
        if (busy) region.setAttribute('aria-busy', 'true');
        else region.removeAttribute('aria-busy');
    }
    const button = $('btn-save');
    if (!button) return;
    if (saveButtonLabel === null) saveButtonLabel = button.innerHTML;
    if (busy) {
        clearTimeout(saveConfirmTimer);
        delete button.dataset.confirmed;
        button.textContent = 'Kaydediliyor…';
    } else {
        restoreSaveButton(button);
    }
}

async function saveSettingsToServer() {
    // Ctrl+S düğmeyi atlar: aynı anda tek kayıt yürür.
    if (saveInFlight) return;
    saveInFlight = true;
    commitDomToActiveConfig();
    const submitted = JSON.stringify(buildPayload());
    const submittedRevision = typeof editRevision === 'number' ? editRevision : null;
    setSaving(true);
    let saved = false;
    try {
        const res = await apiFetch('/api/settings', {
            method: 'POST',
            headers: settingsRevision
                ? { 'Content-Type': 'application/json', 'If-Match': settingsRevision }
                : { 'Content-Type': 'application/json' },
            body: submitted
        });
        if (res.ok) {
            const result = await res.json();
            settingsRevision = (res.headers && res.headers.get('ETag')) || settingsRevision;
            const editedWhileSaving = submittedRevision !== null && editRevision !== submittedRevision;
            const unchanged = !editedWhileSaving && JSON.stringify(buildPayload()) === submitted;
            if (unchanged && result.settings) {
                appSettings = normalizeFromServer(result.settings);
                renderUI();
            }
            // Server sanitization is accepted only when no local edit happened during
            // the request; otherwise the new local data remains in the working model.
            savedSnapshot = unchanged ? JSON.stringify(buildPayload()) : submitted;
            dirty = JSON.stringify(buildPayload()) !== savedSnapshot;
            saved = true;
            updateDirtyUI();
            refreshScope();
            showToast('ayarlar kaydedildi · ' + new Date().toLocaleTimeString('tr-TR'));
        } else if (res.status === 412) {
            // Başka bir sekme arada kaydetti: bu sekmenin değişiklikleri onun üzerine yazılmadı, yerel düzenlemeler kirli kalır.
            showToast('kaydedilmedi · ayarlar başka bir sekmede değiştirildi', true);
            openModal({
                kicker: 'çakışma',
                title: 'Ayarlar başka bir sekmede değiştirildi',
                body: 'Bu sekmedeki değişiklikler, diğer sekmede kaydedilenlerin üzerine yazılmadı. Güncel ayarları yükleyip değişikliklerini yeniden yapabilirsin; yüklemek bu sekmedeki kaydedilmemiş değişiklikleri atar.',
                confirmLabel: 'Güncel ayarları yükle',
                tone: 'primary',
                onConfirm: reloadSettings
            });
        } else {
            // Sunucu doğrulama/gövde hatasını döndüyse mesajı kullanıcıya göster
            // (ör. yazma izni yok: "Permission denied").
            let detail = '';
            try {
                const body = await res.json();
                if (body && body.error) detail = ' · ' + body.error;
            } catch (ignored) { /* gövde JSON değilse yok say */ }
            showToast('kaydedilemedi · HTTP ' + res.status + detail, true);
        }
    } catch (err) {
        showToast('sunucuya bağlanılamadı · ' + err.message, true);
    } finally {
        saveInFlight = false;
        setSaving(false);
        if (saved) confirmSaveInPlace();
    }
}

/* Kaydetme başarısı **yerinde** onaylanır: birincil düğme kısa süre "Kaydedildi" der.
   (Arc: ön plan eyleminin tek kanıtı bildirim olmamalı; bildirim yalnızca destekler.) */
function confirmSaveInPlace() {
    const button = $('btn-save');
    if (!button) return;
    clearTimeout(saveConfirmTimer);
    button.dataset.confirmed = 'true';
    button.textContent = 'Kaydedildi';
    saveConfirmTimer = setTimeout(() => {
        delete button.dataset.confirmed;
        restoreSaveButton(button);
    }, 2400);
}

/* Sunucudaki kayıtlı ayarları geri yükler (kaydedilmemiş değişiklikleri atar). */
async function reloadSettings() {
    await fetchSettings();
    showToast('kayıtlı ayarlar geri yüklendi');
}

function filterDiscordRolesForActiveServer() {
    discordRoles = [];
    rawGuildsList.forEach(guild => {
        if (activeServer === 'global' || guild.guild_id === activeServer) {
            guild.roles.forEach(role => discordRoles.push(Object.assign({}, role, { guild_name: guild.guild_name })));
        }
    });
    renderRolePickerList($('role-search-input') ? $('role-search-input').value : '');
    renderRoleChips();
}

/* ══ ÇİZİM ═════════════════════════════════════════════════════════════════ */

function renderUI() {
    const cfg = getActiveConfig();
    writeModulesDom(cfg.modules);
    writeAiDom(cfg.ai);
    writeEngineDom(getEngineConfig(cfg));

    const filter = cfg.role_filters[currentModule] || { mode: 'disabled', roles: [] };
    qsa('input[name="filter-mode"]').forEach(radio => { radio.checked = radio.value === filter.mode; });
    selectStudioModule(currentModule);

    updateModuleUI();
    updateEngineSummary();
    updateEngineDependencies();
    updateCharCount();
    updatePresetChips();
    updatePlatformHosts();
    updateDirtyUI();
}

/* ══ ONAY PENCERESİ ═══════════════════════════════════════════════════════ */

/* Onay penceresindeki odaklanabilir öğeler (odak tuzağı için). */
function modalFocusables() {
    const card = qs('.modal-card');
    if (!card) return [];
    return Array.prototype.slice.call(
        card.querySelectorAll('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])')
    ).filter(el => !el.disabled && !el.hidden);
}

/* Odak pencere içinde döner: Tab son öğeden ilk öğeye, Shift+Tab tersine. */
function trapModalFocus(e) {
    if (e.key !== 'Tab') return;
    const items = modalFocusables();
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
    }
}

function openModal(options) {
    const modal = $('modal');
    if (!modal) return;
    $('modal-kicker').textContent = options.kicker || 'onay';
    $('modal-title').textContent = options.title || '';
    $('modal-body').textContent = options.body || '';
    const confirm = $('modal-confirm');
    confirm.textContent = options.confirmLabel || 'Onayla';
    confirm.classList.toggle('btn-danger', options.tone !== 'primary');
    modalAction = options.onConfirm || null;
    modalReturnFocus = document.activeElement;
    modal.hidden = false;
    confirm.focus();
}

function closeModal() {
    const modal = $('modal');
    if (modal) modal.hidden = true;
    modalAction = null;
    // Odak, pencereyi açan öğeye döner: klavye kullanıcısı sayfada kaybolmaz.
    if (
        modalReturnFocus &&
        typeof modalReturnFocus.focus === 'function' &&
        document.contains(modalReturnFocus)
    ) {
        modalReturnFocus.focus();
    }
    modalReturnFocus = null;
}

function requestReload(title, body) {
    if (!dirty) {
        reloadSettings();
        return;
    }
    openModal({
        kicker: 'kaydedilmemiş değişiklik',
        title: title,
        body: body,
        confirmLabel: 'Değişiklikleri at',
        onConfirm: reloadSettings
    });
}

/* ══ BİLDİRİM ═════════════════════════════════════════════════════════════ */

function showToast(message, isError) {
    const toast = $('toast');
    if (!toast) return;
    const life = isError ? 5200 : 3200;
    $('toast-msg').textContent = message;
    if ($('toast-led')) $('toast-led').dataset.state = isError ? 'down' : 'live';
    toast.dataset.tone = isError ? 'error' : 'ok';
    toast.style.setProperty('--toast-life', life + 'ms');
    // Animasyonu yeniden başlatmak için sınıfı sıfırla (önce reflow, sonra ekle).
    toast.classList.remove('is-open');
    void toast.offsetWidth;
    toast.classList.add('is-open');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast.classList.remove('is-open'), life);
}

/* ══ KAYDIRMA KONUMUNA GÖRE BÖLÜM VURGUSU ═════════════════════════════════ */

/* Aktif bölüm işaretini tek yerden uygular: görsel (`is-active`) + erişilebilirlik
   (`aria-current`). Durum yalnızca renkle taşınmaz. */
function setActiveNav(sectionId) {
    qsa('[data-nav]').forEach(link => {
        const on = link.dataset.nav === sectionId;
        link.classList.toggle('is-active', on);
        if (on) link.setAttribute('aria-current', 'true');
        else link.removeAttribute('aria-current');
    });
}

function initScrollspy() {
    if (!('IntersectionObserver' in window)) return;
    const observer = new IntersectionObserver(entries => {
        entries.forEach(entry => {
            if (!entry.isIntersecting) return;
            setActiveNav(entry.target.id);
        });
    }, { rootMargin: '-40% 0px -55% 0px', threshold: 0 });
    qsa('.sheet').forEach(section => observer.observe(section));
    // Açılışta HTML'deki başlangıç işaretiyle tutarlı kal.
    const first = qs('[data-nav].is-active');
    if (first) setActiveNav(first.dataset.nav);
}

/* ══ OLAYLAR ══════════════════════════════════════════════════════════════ */

function bind() {
    // Ayarlar satırları: anahtarın adı/açıklaması satırdaki metinlerden gelir
    wireSwitchLabels();

    // Modül anahtarları
    MODULE_KEYS.forEach(key => {
        const box = $('mod-' + key);
        if (!box) return;
        box.addEventListener('change', () => {
            onConfigModified('modules');
            updateModuleUI();
            updatePlatformHosts();
        });
    });

    // Modül satırından ilgili rol filtresine atla (bölümler arası bağlantı)
    qsa('[data-jump-module]').forEach(button => button.addEventListener('click', () => {
        selectStudioModule(button.dataset.jumpModule);
        scrollToSection('sec-role-filters');
        const tab = $('modtab-' + button.dataset.jumpModule);
        if (tab) tab.focus({ preventScroll: true });
    }));

    // AI persona
    if ($('ai-model-box')) $('ai-model-box').addEventListener('input', () => {
        onConfigModified('ai');
        updateNavCounts();
        updatePresetChips();
    });
    if ($('ai-prompt-box')) $('ai-prompt-box').addEventListener('input', () => {
        onConfigModified('ai');
        updateCharCount();
    });
    qsa('[data-model-preset]').forEach(button => button.addEventListener('click', () => setModelPreset(button.dataset.modelPreset)));

    // Motor
    qsa('[data-engine-switch]').forEach(el => el.addEventListener('change', () => {
        onConfigModified('engine');
        updateEngineSummary();
        updateEngineDependencies();
    }));
    qsa('[data-engine-range]').forEach(el => el.addEventListener('input', () => {
        onConfigModified('engine');
        updateEngineSummary();
    }));
    qsa('[data-emoji-key]').forEach(el => el.addEventListener('input', () => onConfigModified('engine')));
    if ($('engine-twitter-source')) $('engine-twitter-source').addEventListener('change', () => {
        onConfigModified('engine');
        updateEngineSummary();
    });
    if ($('engine-sensitive-mode')) $('engine-sensitive-mode').addEventListener('change', () => onConfigModified('engine'));
    if ($('engine-translation-target')) $('engine-translation-target').addEventListener('input', () => onConfigModified('engine'));
    if ($('channel-select')) $('channel-select').addEventListener('change', onChannelChange);
    if ($('btn-retry-channels')) $('btn-retry-channels').addEventListener('click', retryDiscordChannels);

    // Rol stüdyosu: sekmeler (ok tuşlarıyla gezinme dahil)
    const tablist = qs('.studio-list');
    if (tablist) tablist.addEventListener('keydown', onStudioKeydown);
    qsa('[data-studio-module]').forEach(button => button.addEventListener('click', () => selectStudioModule(button.dataset.studioModule)));

    qsa('input[name="filter-mode"]').forEach(radio => radio.addEventListener('change', () => {
        const filter = getCurFilter();
        filter.mode = radio.value;
        onConfigModified('role_filters');
        updateFilterModeUI(radio.value);
        updateModuleBadges();
        updateNavCounts();
    }));

    if ($('btn-toggle-picker')) $('btn-toggle-picker').addEventListener('click', toggleRolePicker);
    if ($('role-search-input')) {
        $('role-search-input').addEventListener('input', () => {
            pickerCursor = -1;
            renderRolePickerList($('role-search-input').value);
        });
        $('role-search-input').addEventListener('keydown', onPickerKeydown);
    }
    if ($('role-input-box')) $('role-input-box').addEventListener('keydown', e => {
        if (e.key === 'Enter') {
            e.preventDefault();
            addRoleFromInput();
        }
    });
    if ($('btn-add-role')) $('btn-add-role').addEventListener('click', addRoleFromInput);

    // Kapsam
    if ($('server-select')) $('server-select').addEventListener('change', onServerChange);
    if ($('btn-toggle-custom')) $('btn-toggle-custom').addEventListener('click', toggleServerCustomSettings);

    // Bölüm bazlı "genele sıfırla" (yalnızca sunucu kapsamında, sabitlenmiş bölümde görünür)
    qsa('[data-block-reset]').forEach(button => button.addEventListener('click', () => resetBlockToGlobal(button.dataset.blockReset)));

    // Kaydet / geri yükle / vazgeç
    ['btn-save', 'btn-save-dock'].forEach(id => {
        const el = $(id);
        if (el) el.addEventListener('click', saveSettingsToServer);
    });
    if ($('btn-reload')) $('btn-reload').addEventListener('click', () => requestReload(
        'Kayıtlı ayarları geri yükle',
        'Sunucudaki kayıtlı ayarlar yeniden yüklenecek; kaydedilmemiş tüm değişiklikler kaybolacak.'
    ));
    if ($('btn-discard')) $('btn-discard').addEventListener('click', () => requestReload(
        'Değişiklikleri at',
        'Kaydedilmemiş değişiklikler silinecek ve sunucudaki kayıtlı ayarlar geri yüklenecek.'
    ));

    // Onay penceresi: odak tuzağı (Tab pencerede döner, Escape kapatır, kapanınca odak döner)
    if ($('modal')) $('modal').addEventListener('keydown', trapModalFocus);

    // Onay penceresi
    if ($('modal-confirm')) $('modal-confirm').addEventListener('click', () => {
        const action = modalAction;
        closeModal();
        if (action) action();
    });
    qsa('[data-modal-close]').forEach(el => el.addEventListener('click', closeModal));

    // Açılır menü: dışarı tıklama ve klavye
    document.addEventListener('click', onDocumentClick);
    window.addEventListener('keydown', onGlobalKeydown);

    // Kaydedilmemiş değişiklik varken sekmeden çıkışta uyar.
    window.addEventListener('beforeunload', e => {
        if (!dirty) return;
        e.preventDefault();
        e.returnValue = '';
    });
}

function onStudioKeydown(e) {
    const buttons = qsa('[data-studio-module]');
    const current = buttons.map(b => b.dataset.studioModule).indexOf(currentModule);
    let next = -1;
    if (e.key === 'ArrowDown' || e.key === 'ArrowRight') next = (current + 1) % buttons.length;
    else if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') next = (current - 1 + buttons.length) % buttons.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = buttons.length - 1;
    if (next < 0) return;
    e.preventDefault();
    const target = buttons[next];
    selectStudioModule(target.dataset.studioModule);
    target.focus();
}

function onDocumentClick(e) {
    const pop = $('role-picker-dropdown');
    if (!pop || pop.hidden) return;
    const inside = e.target instanceof Element && e.target.closest('.picker');
    if (!inside) closeRolePicker();
}

function onGlobalKeydown(e) {
    if (e.key === 'Escape') {
        if ($('modal') && !$('modal').hidden) {
            closeModal();
            return;
        }
        closeRolePicker();
        return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        saveSettingsToServer();
    }
}

/* ══ AÇILIŞ ═══════════════════════════════════════════════════════════════ */

async function boot() {
    $('btn-logout').addEventListener('click', async () => {
        if (dirty && !window.confirm('Kaydedilmemiş değişiklikler var. Çıkış yapılsın mı?')) return;
        try {
            const response = await apiFetch('/api/auth/logout', { method: 'POST' });
            if (!response.ok) throw new Error('Çıkış yapılamadı');
            window.location.replace('/login');
        } catch (error) { showToast(error.message, true); }
    });
    bind();
    initScrollspy();
    refreshScope();
    renderUI();
    // .env kaynaklı adresler ve sürüm bilgisi önce gelir: ipuçları bunları kullanır.
    await fetchInfo();
    await fetchSettings();
    fetchDiscordRoles();
    fetchDiscordChannels();
    checkHealth();
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
} else {
    boot();
}
