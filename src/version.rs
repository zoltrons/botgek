//! Sürüm bilgisi — **tek kaynak**.
//!
//! Sürüm, git etiketlerinden türetilir ve **derleme zamanında** ikiliye gömülür (`build.rs`,
//! `cargo:rustc-env`). Türetme mantığı `build-support/version-parse.rs` içindedir ve hem
//! `build.rs` hem bu modül tarafından `include!` edilir: kopya mantık yoktur, gömülen değer
//! ile çalışma zamanında raporlanan değer ayrışamaz (test bunu doğrular).
//!
//! Sözleşme:
//! - `SEMVER` → `0.2.0` (API/bildirimler; `Cargo.toml` ile senkron, etiket varsa etiketten).
//! - `DISPLAY` → `0.2.0+5.g8fc2692` (etiketten sonraki 5 commit) veya yayında yalın `0.2.0`.
//! - `FULL` → `v0.2.0+5.g8fc2692` (panelde gösterilen biçim).
//! - `IS_RELEASE` → tam etiket isabeti + temiz ağaç.
//!
//! Git bilgisi bulunamazsa (Docker/tarball bağlamı) `Cargo.toml` sürümüne düşülür ve
//! geliştirme işareti eklenir (`0.2.0+local`); derleme asla bu yüzden başarısız olmaz.

use std::sync::LazyLock;

include!("../build-support/version-parse.rs");

/// Derleme çıktısı varsa onu, yoksa (`build.rs` çalışmadıysa) güvenli yedeği kullanır.
macro_rules! build_or {
    ($key:literal, $fallback:expr) => {
        match option_env!($key) {
            Some(value) => value,
            None => $fallback,
        }
    };
}

/// `build.rs`'in yazdığı `"true"`/`"false"` bayrağını derleme zamanında okur.
///
/// Not: const bağlamda `&str` eşitliği henüz kararlı olmadığı için bayt karşılaştırması
/// yapılır (dış bağımlılık yok, `PartialEq` gerektirmez).
const fn build_flag(value: Option<&str>) -> bool {
    match value {
        Some(text) => matches!(text.as_bytes(), [b't', b'r', b'u', b'e']),
        None => false,
    }
}

/// Semver taban sürüm (ör. `0.2.0`) — `/api/info` ve bildirimler bu değeri taşır.
pub const SEMVER: &str = build_or!("BOTGEK_VERSION_SEMVER", env!("CARGO_PKG_VERSION"));
/// Tam sürüm gösterimi (ör. `0.2.0+5.g8fc2692.dirty`).
pub const DISPLAY: &str = build_or!("BOTGEK_VERSION_DISPLAY", env!("CARGO_PKG_VERSION"));
/// `v` önekli tam sürüm (panelde gösterilir).
pub const FULL: &str = build_or!(
    "BOTGEK_VERSION_FULL",
    concat!("v", env!("CARGO_PKG_VERSION"))
);
/// Kısaltılmış commit kimliği, bulunamazsa `unknown`.
pub const GIT_SHA: &str = build_or!("BOTGEK_GIT_SHA", "unknown");
/// En yakın etiket (ör. `v0.2.0`); etiket yoksa boş.
pub const GIT_TAG: &str = build_or!("BOTGEK_GIT_TAG", "");
/// Etiketten bu yana commit sayısı; etiket yoksa boş (`commits_since_tag()` ile sayıya çevrilir).
pub const COMMITS_SINCE_TAG: &str = build_or!("BOTGEK_COMMITS_SINCE_TAG", "");
/// Derleme zamanı (UTC, `SOURCE_DATE_EPOCH` ile tekrarlanabilir).
pub const BUILD_DATE: &str = build_or!("BOTGEK_BUILD_DATE", "unknown");
/// Derlenen hedef üçlü (ör. `aarch64-unknown-linux-gnu`).
pub const TARGET: &str = build_or!("BOTGEK_TARGET", "unknown");
/// Tam etiket isabeti + temiz ağaç → yayın sürümü.
pub const IS_RELEASE: bool = build_flag(option_env!("BOTGEK_IS_RELEASE"));
/// Derleme sırasında çalışma ağacı kirli miydi?
pub const IS_DIRTY: bool = build_flag(option_env!("BOTGEK_IS_DIRTY"));

/// Dış servislere kendini tanıtan User-Agent (`media`, `reddit_video` vb. kullanır).
pub const USER_AGENT: &str = build_or!(
    "BOTGEK_USER_AGENT",
    "Botgek/unknown (Discord link converter)"
);
/// Discord REST API için resmi standartta User-Agent (`DiscordBot ($url, $versionNumber)`).
pub const DISCORD_USER_AGENT: &str = build_or!(
    "BOTGEK_DISCORD_USER_AGENT",
    "DiscordBot (https://github.com/zoltrons/botgek)"
);

/// Derlemeye gömülen **ham** git girdilerinden sürümün yeniden çözülmüş hâli.
///
/// Gömülen sabitlerle aynı fonksiyondan (`derive_version`) üretildiği için tutarlıdır;
/// teşhis logu ve `/api/info` bu zengin görünümü kullanır.
static PROVENANCE: LazyLock<DerivedVersion> = LazyLock::new(|| {
    derive_version(&RawVersionInputs {
        pkg_version: env!("CARGO_PKG_VERSION"),
        override_version: non_empty(option_env!("BOTGEK_VERSION_OVERRIDE")),
        describe: non_empty(option_env!("BOTGEK_GIT_DESCRIBE")),
        git_sha: non_empty(option_env!("BOTGEK_GIT_SHA")),
    })
});

/// `Some("")`/`Some("  ")` değerlerini `None` sayar.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|inner| !inner.is_empty())
}

/// Sürümün yeniden çözülmüş (zengin) görünümü: commit sayısı, etiket, kirli ağaç bayrağı.
pub fn provenance() -> &'static DerivedVersion {
    &PROVENANCE
}

/// Etiketten bu yana commit sayısı (etiket yoksa `None`).
pub fn commits_since_tag() -> Option<u32> {
    COMMITS_SINCE_TAG.parse::<u32>().ok()
}

/// En yakın etiket (yoksa `None`).
pub fn tag() -> Option<&'static str> {
    if GIT_TAG.is_empty() {
        None
    } else {
        Some(GIT_TAG)
    }
}

/// Tek satır teşhis özeti: sürüm · yayın/geliştirme · sha · etiket · tarih · hedef.
pub fn summary() -> String {
    let details = provenance();
    let tag = match (&details.tag, details.commits_since_tag) {
        (Some(tag), Some(count)) => format!("{tag}+{count}"),
        (Some(tag), None) => tag.clone(),
        (None, _) => "etiket yok".to_string(),
    };
    format!(
        "{} · {} · {} · {} · {} · {}{}",
        details.full,
        if details.is_release {
            "yayın"
        } else {
            "geliştirme"
        },
        details.git_sha.as_deref().unwrap_or("unknown"),
        tag,
        BUILD_DATE,
        TARGET,
        if details.is_dirty {
            " · kirli ağaç"
        } else {
            ""
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(
        pkg: &'a str,
        override_version: Option<&'a str>,
        describe: Option<&'a str>,
        sha: Option<&'a str>,
    ) -> RawVersionInputs<'a> {
        RawVersionInputs {
            pkg_version: pkg,
            override_version,
            describe,
            git_sha: sha,
        }
    }

    /// Etiketin tam üstünde: yalın sürüm, build metadata yok.
    #[test]
    fn derive_release_on_tag_is_plain() {
        let derived = derive_version(&inputs("0.2.0", None, Some("v0.2.0-0-g8fc2692"), None));
        assert_eq!(derived.semver, "0.2.0");
        assert_eq!(derived.display, "0.2.0");
        assert_eq!(derived.full, "v0.2.0");
        assert!(derived.is_release);
        assert!(!derived.is_dirty);
        assert_eq!(derived.commits_since_tag, Some(0));
        assert_eq!(derived.tag.as_deref(), Some("v0.2.0"));
    }

    /// Etiketten sonraki commit'ler: commit sayısı + kısa sha metadata'ya girer.
    #[test]
    fn derive_after_tag_counts_commits() {
        let derived = derive_version(&inputs("0.2.0", None, Some("v0.2.0-5-g8fc2692"), None));
        assert_eq!(derived.semver, "0.2.0");
        assert_eq!(derived.display, "0.2.0+5.g8fc2692");
        assert_eq!(derived.full, "v0.2.0+5.g8fc2692");
        assert!(!derived.is_release);
        assert_eq!(derived.commits_since_tag, Some(5));
    }

    /// Kirli çalışma ağacı `.dirty` olarak işaretlenir ve yayın sayılmaz.
    #[test]
    fn derive_dirty_tree_is_marked() {
        let derived = derive_version(&inputs(
            "0.2.0",
            None,
            Some("v0.2.0-0-g8fc2692-dirty"),
            None,
        ));
        assert_eq!(derived.display, "0.2.0+dev.g8fc2692.dirty");
        assert!(!derived.is_release);
        assert!(derived.is_dirty);
    }

    /// Hiç etiket yoksa geliştirme sürümü üretilir (`+dev.g<sha>`).
    #[test]
    fn derive_without_tags_uses_dev_marker() {
        let derived = derive_version(&inputs("0.2.0", None, Some("8fc2692"), None));
        assert_eq!(derived.semver, "0.2.0");
        assert_eq!(derived.display, "0.2.0+dev.g8fc2692");
        assert!(!derived.is_release);
        assert_eq!(derived.tag, None);
    }

    /// Git hiç yoksa (Docker/tarball) `Cargo.toml` sürümü + `local` işareti kullanılır.
    #[test]
    fn derive_without_git_falls_back_to_package_version() {
        let derived = derive_version(&inputs("0.2.0", None, None, None));
        assert_eq!(derived.semver, "0.2.0");
        assert_eq!(derived.display, "0.2.0+local");
        assert!(!derived.is_release);
        assert_eq!(derived.git_sha, None);
    }

    /// Açık override (CI/Docker) etiketten de `Cargo.toml`'dan da önceliklidir.
    #[test]
    fn derive_override_wins_over_tag_and_package() {
        let derived = derive_version(&inputs(
            "0.1.0",
            Some("v0.3.0"),
            Some("v0.2.0-4-g8fc2692"),
            None,
        ));
        assert_eq!(derived.semver, "0.3.0", "`v` öneki temizlenmeli");
        assert_eq!(derived.commits_since_tag, Some(4));
    }

    /// Git yokken override yalnızca `Cargo.toml` ile aynıysa yayın sayılır (sürüm senkronu).
    #[test]
    fn derive_override_without_git_requires_package_sync() {
        assert!(derive_version(&inputs("0.2.0", Some("v0.2.0"), None, None)).is_release);
        assert!(
            !derive_version(&inputs("0.1.0", Some("v0.2.0"), None, None)).is_release,
            "Cargo.toml güncellenmemişse yayın sayılmamalı"
        );
    }

    /// `refs/tags/v1.2.3` gibi ham etiket girdisi de kabul edilir.
    #[test]
    fn derive_normalizes_raw_tag_input() {
        let derived = derive_version(&inputs("0.0.0", Some("refs/tags/v1.2.3"), None, None));
        assert_eq!(derived.semver, "1.2.3");
    }

    /// Geçersiz `Cargo.toml` sürümü güvenli tabana düşer (panik yok).
    #[test]
    fn derive_invalid_package_version_falls_back() {
        let derived = derive_version(&inputs("bilinmeyen", None, None, None));
        assert_eq!(derived.semver, "0.0.0");
        assert!(is_semver(&derived.semver));
    }

    /// Ön sürümlü etiketler (`v1.0.0-rc.1`) bozulmadan geçer.
    #[test]
    fn derive_keeps_prerelease_tags() {
        let derived = derive_version(&inputs("1.0.0", None, Some("v1.0.0-rc.1-2-g8fc2692"), None));
        assert_eq!(derived.semver, "1.0.0-rc.1");
        assert_eq!(derived.display, "1.0.0-rc.1+2.g8fc2692");
    }

    /// Zaman damgası çevirimi bilinen değerlerle doğrulanır (dış bağımlılık yok).
    #[test]
    fn format_unix_utc_matches_known_dates() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_unix_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}

#[cfg(test)]
mod compiled {
    use super::*;

    /// **Derleme ↔ çalışma zamanı tutarlılığı:** gömülen sabitler, ham girdilerden yeniden
    /// çözülen değerlerle birebir aynı olmalı (kopya mantık ayrışmasını yakalar).
    #[test]
    fn compiled_constants_match_runtime_provenance() {
        let derived = provenance();
        assert_eq!(
            SEMVER, derived.semver,
            "gömülen semver çalışma zamanıyla ayrışık"
        );
        assert_eq!(DISPLAY, derived.display, "gömülen gösterim ayrışık");
        assert_eq!(FULL, derived.full, "gömülen tam sürüm ayrışık");
        assert_eq!(IS_RELEASE, derived.is_release, "yayın bayrağı ayrışık");
        assert_eq!(IS_DIRTY, derived.is_dirty, "kirli ağaç bayrağı ayrışık");
        assert_eq!(GIT_SHA, derived.git_sha.as_deref().unwrap_or("unknown"));
        assert_eq!(GIT_TAG, derived.tag.as_deref().unwrap_or(""));
    }

    /// Gömülen değerlerin biçimi (etiket/sürüm güncellemesi bozmasın).
    #[test]
    fn compiled_constants_have_expected_shape() {
        assert!(is_semver(SEMVER), "semver geçersiz: {SEMVER}");
        assert!(
            DISPLAY.starts_with(SEMVER),
            "gösterim semver ile başlamalı: {DISPLAY}"
        );
        assert_eq!(FULL, format!("v{DISPLAY}"));
        assert!(USER_AGENT.starts_with("Botgek/"), "UA biçimi: {USER_AGENT}");
        assert!(
            DISCORD_USER_AGENT.starts_with("DiscordBot (https://"),
            "UA biçimi"
        );
        assert!(
            DISCORD_USER_AGENT.contains(SEMVER),
            "Discord UA semver taşımalı"
        );
        assert!(summary().contains(SEMVER));
        assert!(!BUILD_DATE.is_empty() && !TARGET.is_empty());
    }

    /// Yayın sürümünde build metadata bulunmaz (temiz semver sözleşmesi).
    #[test]
    fn release_display_has_no_build_metadata() {
        if IS_RELEASE {
            assert_eq!(DISPLAY, SEMVER, "yayın sürümünde metadata olmamalı");
        }
    }
}
