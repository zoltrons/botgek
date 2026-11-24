//! Letterboxd takip akışı: takip edilen hesapların RSS akışını yoklar, yeni günlük kayıtlarını
//! (izleme/inceleme) takip eden kanallara klasik embed kartı olarak gönderir.
//!
//! Hız: akışlar tek tek, istekler arasında en az [`REQUEST_GAP`] beklenerek çekilir; böylece
//! takip sayısı ne olursa olsun Letterboxd'a giden hız sabit kalır. Her hesap en sık
//! [`MIN_ROUND`]'da bir yoklanır; hesap çoğaldıkça tur kendiliğinden uzar. Letterboxd RSS'i
//! önbelleklemez (`no-store`) ve hız sınırı başlığı vermez; ölçümde ~2,3 istek/sn'de 429
//! görülmedi. 429/403/503 gelirse aralık ikiye katlanır ([`MAX_GAP`]'e dek), başarıda
//! yarıya iner. Atlanan tur kayıp değildir: kanal başına son gönderilen kayıt kimliği saklanır.

use std::time::Duration;

use tracing::{debug, info, warn};

use crate::converters::builtin::letterboxd::{self, DiaryEntry, DiaryError};
use crate::converters::builtin::raw::{self, DiscordError};
use crate::store::{self, LetterboxdFollow};

/// Ardışık iki RSS isteği arasındaki en kısa süre (ölçülen güvenli hızın yarısından azı).
const REQUEST_GAP: Duration = Duration::from_secs(1);
/// Geri çekilmede ulaşılabilecek en uzun istek aralığı.
const MAX_GAP: Duration = Duration::from_secs(60);
/// Bir hesabın iki yoklaması arasındaki en kısa süre.
const MIN_ROUND: Duration = Duration::from_secs(30);
/// Uzun aradan (kesinti, geri çekilme) sonra bir kanala tek turda gönderilen en fazla kayıt.
const MAX_POSTS_PER_ROUND: usize = 5;

/// Yoklama döngüsünü arka planda başlatır (süreç başına bir kez).
pub fn spawn(client: reqwest::Client, token: String) {
    tokio::spawn(async move {
        info!("Letterboxd takip akışı başladı");
        let mut gap = REQUEST_GAP;
        loop {
            let started = tokio::time::Instant::now();
            let follows = store::blocking(|| store::letterboxd_follows(None)).await;
            // Sorgu kullanıcı adına göre sıralı: aynı hesabın kanalları ardışık gelir.
            for subs in follows.chunk_by(|a, b| a.username == b.username) {
                let user = &subs[0].username;
                match letterboxd::diary(&client, user).await {
                    Ok(entries) => {
                        gap = (gap / 2).max(REQUEST_GAP);
                        for sub in subs {
                            deliver(&client, &token, sub, &entries).await;
                        }
                    }
                    Err(DiaryError::Throttled) => {
                        gap = (gap * 2).min(MAX_GAP);
                        warn!(
                            gap_secs = gap.as_secs(),
                            "Letterboxd akışı yavaşlatıldı (hız sınırı)"
                        );
                    }
                    Err(DiaryError::NotFound) => debug!("Letterboxd hesabı yok: {user}"),
                    Err(DiaryError::Failed(reason)) => {
                        debug!("Letterboxd akışı okunamadı ({user}): {reason}")
                    }
                }
                tokio::time::sleep(gap).await;
            }
            tokio::time::sleep_until(started + MIN_ROUND).await;
        }
    });
}

/// `last` sonrası kayıtlar, eskiden yeniye; en fazla son [`MAX_POSTS_PER_ROUND`] tanesi.
fn unseen(entries: &[DiaryEntry], last: u64) -> Vec<&DiaryEntry> {
    let mut new: Vec<&DiaryEntry> = entries.iter().filter(|entry| entry.id > last).collect();
    new.sort_by_key(|entry| entry.id);
    new.split_off(new.len().saturating_sub(MAX_POSTS_PER_ROUND))
}

async fn deliver(
    client: &reqwest::Client,
    token: &str,
    sub: &LetterboxdFollow,
    entries: &[DiaryEntry],
) {
    let new = unseen(entries, sub.last_entry);
    let Some(newest) = new.last().map(|entry| entry.id) else {
        return;
    };
    let mut seen = sub.last_entry;
    let mut gone = false;
    for entry in new {
        let result = raw::send_channel_message(
            client,
            token,
            sub.channel_id,
            &entry.message(client).await,
            &[],
        )
        .await;
        if let Err(err) = &result {
            warn!(
                channel = %sub.channel_id,
                user = %sub.username,
                "Letterboxd kaydı kanala gönderilemedi: {err}"
            );
        }
        gone = channel_gone(result.as_ref().err());
        let (next, stop) = after_send(result.err().as_ref(), seen, entry.id, newest);
        seen = next;
        if stop {
            break;
        }
    }
    // Kanal silinmiş (404): takip kalırsa her turda yoklanır, sunucu sınırını doldurur ve
    // `/letterboxd bırak` artık o kanalda çalıştırılamaz.
    if gone {
        let (channel, user) = (sub.channel_id, sub.username.clone());
        store::blocking(move || store::unfollow_letterboxd(channel, &user)).await;
        info!(channel = %sub.channel_id, user = %sub.username, "Kanal yok; Letterboxd takibi kaldırıldı");
        return;
    }
    if seen > sub.last_entry {
        let (channel, user) = (sub.channel_id, sub.username.clone());
        store::blocking(move || store::set_letterboxd_last_entry(channel, &user, seen)).await;
    }
}

/// Kanal silinmiş mi? 403 sayılmaz: izin geçici olarak kaldırılmış olabilir.
fn channel_gone(err: Option<&DiscordError>) -> bool {
    matches!(err, Some(DiscordError::Rejected(404)))
}

/// Gönderim sonucuna göre kanalın yeni son kaydı ve turun durup durmayacağı.
fn after_send(err: Option<&DiscordError>, seen: u64, id: u64, newest: u64) -> (u64, bool) {
    match err {
        // Belirsiz sonuç yeniden gönderilmez: kart iki kez düşebilirdi.
        None | Some(DiscordError::Ambiguous | DiscordError::InvalidPayload) => (id, false),
        // Kanal silinmiş ya da yetki yok: bekleyen kayıtlar atlanır, her turda yeniden denenmez.
        Some(DiscordError::Rejected(_)) => (newest, true),
        // Geçici (hız sınırı, Discord erişilemez): kalanlar sonraki turda denenir.
        Some(_) => (seen, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64) -> DiaryEntry {
        DiaryEntry::stub(id)
    }

    #[test]
    fn unseen_keeps_newest_entries_oldest_first() {
        let entries: Vec<DiaryEntry> = (1..=9).rev().map(entry).collect();
        let ids: Vec<u64> = unseen(&entries, 2).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![5, 6, 7, 8, 9]);
        assert!(unseen(&entries, 9).is_empty());
    }

    #[test]
    fn send_outcome_advances_skips_or_retries() {
        assert_eq!(after_send(None, 1, 2, 5), (2, false));
        assert_eq!(
            after_send(Some(&DiscordError::Ambiguous), 1, 2, 5),
            (2, false)
        );
        assert_eq!(
            after_send(Some(&DiscordError::Rejected(403)), 1, 2, 5),
            (5, true)
        );
        assert_eq!(
            after_send(Some(&DiscordError::Unavailable), 1, 2, 5),
            (1, true)
        );
        let limited = DiscordError::RateLimited(Duration::from_secs(1));
        assert_eq!(after_send(Some(&limited), 1, 2, 5), (1, true));

        assert!(channel_gone(Some(&DiscordError::Rejected(404))));
        assert!(!channel_gone(Some(&DiscordError::Rejected(403))));
        assert!(!channel_gone(None));
    }
}
