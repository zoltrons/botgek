// Sürüm türetme mantığı — **TEK KAYNAK**.
//
// Bu dosya hem `build.rs` (derleme zamanı) hem `src/version.rs` (çalışma zamanı) tarafından
// `include!` edilir: aynı girdiler → aynı sonuç. Kopya mantık yoktur, dış bağımlılık yoktur.
//
// Sözleşme (semver):
//   * `Cargo.toml` sürümü **taban** sürümdür; release script'i onu güncelleyip `vX.Y.Z`
//     etiketi atar.
//   * Etiketin tam üstünde ve çalışma ağacı temizse → sürüm yalın `0.2.0` olur (`is_release`).
//   * Etiketten sonraki commit'lerde → build metadata eklenir: `0.2.0+5.g8fc2692`
//     (5 commit, kısa sha). Kirli ağaçta `.dirty` eklenir.
//   * Hiç etiket yoksa → `0.2.0+dev.g8fc2692` (geliştirme sürümü).
//   * Git/etiket bilgisi hiç yoksa → `0.2.0+local`.

/// Derlemeye gömülen (ve çalışma zamanında yeniden çözülen) ham sürüm girdileri.
#[derive(Debug, Clone, Default)]
pub struct RawVersionInputs<'a> {
    /// `Cargo.toml` sürümü — sürümün tabanı.
    pub pkg_version: &'a str,
    /// Açık sürüm override'ı (`BOTGEK_BUILD_VERSION`; CI tag derlemesinde `v0.2.0`).
    pub override_version: Option<&'a str>,
    /// `git describe --tags --long --dirty --always --match "v[0-9]*"` çıktısı.
    pub describe: Option<&'a str>,
    /// `git rev-parse --short HEAD` (describe yoksa kullanılır).
    pub git_sha: Option<&'a str>,
}

/// Ham girdilerden türetilen sürüm bilgisi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedVersion {
    /// Semver taban (ör. `0.2.0`) — API ve bildirimlerde bu kullanılır.
    pub semver: String,
    /// Tam gösterim (ör. `0.2.0+5.g8fc2692.dirty`).
    pub display: String,
    /// `v` önekli tam gösterim (ör. `v0.2.0+5.g8fc2692`).
    pub full: String,
    /// Tam etiket isabeti + temiz ağaç → yayın sürümü.
    pub is_release: bool,
    /// Çalışma ağacı kirli mi (`git describe` `-dirty`)?
    pub is_dirty: bool,
    /// Etiketten bu yana commit sayısı (etiket yoksa `None`).
    pub commits_since_tag: Option<u32>,
    /// En yakın etiket (ör. `v0.2.0`), yoksa `None`.
    pub tag: Option<String>,
    /// Kısaltılmış commit kimliği, yoksa `None`.
    pub git_sha: Option<String>,
}

/// `git describe` çıktısının çözümlenmiş hâli.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DescribeParts {
    tag: Option<String>,
    commits: Option<u32>,
    sha: Option<String>,
    dirty: bool,
}

/// `git describe --tags --long --dirty --always --match "v[0-9]*"` çıktısını çözümler.
///
/// Kabul edilen biçimler: `v0.2.0-5-g8fc2692`, `v0.2.0-0-g8fc2692-dirty`, `8fc2692`
/// (etiket yok) ve `8fc2692-dirty`. Başka her şey `Default` (boş) döner.
fn parse_describe(raw: &str) -> DescribeParts {
    let raw = raw.trim();
    let (rest, dirty) = match raw.strip_suffix("-dirty") {
        Some(head) => (head, true),
        None => (raw, false),
    };

    // "…-<n>-g<sha>" soneki: etiketli sürüm biçimi.
    if let Some(index) = rest.rfind("-g") {
        let (left, sha_part) = rest.split_at(index);
        let sha = &sha_part[2..];
        if !left.is_empty() && is_hex(sha) {
            // "v0.2.0-5" → (tag "v0.2.0", commits 5)
            if let Some(dash) = left.rfind('-') {
                let (tag, count) = left.split_at(dash);
                if let Ok(commits) = count[1..].parse::<u32>() {
                    return DescribeParts {
                        tag: Some(tag.to_string()),
                        commits: Some(commits),
                        sha: Some(sha.to_string()),
                        dirty,
                    };
                }
            }
        }
    }

    // Etiket yok: `git describe --always` yalnızca kısaltılmış SHA basar.
    if is_hex(rest) {
        return DescribeParts {
            tag: None,
            commits: None,
            sha: Some(rest.to_string()),
            dirty,
        };
    }

    DescribeParts {
        tag: None,
        commits: None,
        sha: None,
        dirty,
    }
}

/// Yalnızca küçük harfli hex karakterlerden oluşuyor mu (boş hariç)?
fn is_hex(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Sürüm dizgesini normalize eder: baştaki `refs/tags/` ve `v` öneki atılır, semver
/// biçimi doğrulanır. Geçersizse boş dizge döner.
fn normalize_semver(raw: &str) -> String {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("refs/tags/").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix('v').or_else(|| trimmed.strip_prefix('V')).unwrap_or(trimmed);
    if is_semver(trimmed) {
        trimmed.to_string()
    } else {
        String::new()
    }
}

/// `MAJOR.MINOR.PATCH` (+ isteğe bağlı `-ön-sürüm`) biçiminde mi?
fn is_semver(value: &str) -> bool {
    let (core, pre) = match value.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (value, None),
    };
    if let Some(pre) = pre {
        if pre.is_empty() || !pre.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
            return false;
        }
    }
    let mut parts = core.split('.');
    let (major, minor, patch) = (parts.next(), parts.next(), parts.next());
    match (major, minor, patch, parts.next()) {
        (Some(a), Some(b), Some(c), None) => {
            !a.is_empty()
                && !b.is_empty()
                && !c.is_empty()
                && [a, b, c].iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
        }
        _ => false,
    }
}

/// Unix zaman damgasını `YYYY-MM-DDTHH:MM:SSZ` (UTC) biçimine çevirir.
///
/// Tarih hesabı **Howard Hinnant'ın `civil_from_days`** algoritmasıyla yapılır: `chrono`/`time`
/// gibi bir bağımlılık gerekmez ve `SOURCE_DATE_EPOCH` ile **tekrarlanabilir derleme**
/// desteklenir (aynı girdi → aynı damga).
///
/// `build.rs` bunu üretimde kullanır; kütüphane tarafında damga zaten gömülü geldiği için
/// yalnızca testler çağırır (çevirim bilinen tarihlerle doğrulanır).
#[allow(dead_code)]
pub fn format_unix_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);

    // civil_from_days: 1970-01-01 = gün 0.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    if month <= 2 {
        year += 1;
    }

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Ham girdilerden sürüm bilgisini türetir (saf; I/O yok).
///
/// **Tek kaynak:** `build.rs` (derleme zamanı) ve `src/version.rs` (çalışma zamanı) aynı
/// fonksiyonu çağırır → gömülen sürüm ile çalışma zamanında raporlanan sürüm ayrışamaz.
pub fn derive_version(inputs: &RawVersionInputs<'_>) -> DerivedVersion {
    let parts = inputs.describe.map(parse_describe).unwrap_or_default();

    let pkg = normalize_semver(inputs.pkg_version);
    let pkg = if pkg.is_empty() { "0.0.0".to_string() } else { pkg };
    let override_version = inputs
        .override_version
        .map(normalize_semver)
        .filter(|value| !value.is_empty());
    let tag_version = parts
        .tag
        .as_deref()
        .map(normalize_semver)
        .filter(|value| !value.is_empty());

    // Öncelik: açık override (`BOTGEK_BUILD_VERSION`) → en yakın etiket → `Cargo.toml`.
    let semver = match (&override_version, &tag_version) {
        (Some(value), _) => value.clone(),
        (None, Some(value)) => value.clone(),
        (None, None) => pkg.clone(),
    };

    let git_sha = parts.sha.clone().or_else(|| {
        inputs
            .git_sha
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });

    // Yayın sürümü: etiketin tam üstünde ve ağaç temiz. Git bilgisi hiç yoksa (ör. tarball
    // derlemesi) açık override yalnızca `Cargo.toml` ile **aynıysa** yayın kabul edilir:
    // böylece "etiketlendi ama sürüm güncellenmedi" durumu sessizce yayın sayılmaz.
    let is_release = if parts.commits.is_some() {
        !parts.dirty && parts.commits == Some(0)
    } else {
        !parts.dirty && override_version.as_deref() == Some(pkg.as_str())
    };

    // Build metadata: yayın sürümünde **boş** (yalın `0.2.0`), aksi hâlde
    // `<commit sayısı | dev | local>[.g<sha>][.dirty]`.
    let mut metadata: Vec<String> = Vec::new();
    if !is_release {
        if parts.commits.is_none() && git_sha.is_none() {
            metadata.push("local".to_string());
        } else {
            match parts.commits {
                Some(0) | None => metadata.push("dev".to_string()),
                Some(count) => metadata.push(count.to_string()),
            }
            if let Some(sha) = &git_sha {
                metadata.push(format!("g{sha}"));
            }
        }
        if parts.dirty {
            metadata.push("dirty".to_string());
        }
    }

    let display = if metadata.is_empty() {
        semver.clone()
    } else {
        format!("{semver}+{}", metadata.join("."))
    };

    DerivedVersion {
        full: format!("v{display}"),
        display,
        semver,
        is_release,
        is_dirty: parts.dirty,
        commits_since_tag: parts.commits,
        tag: parts.tag,
        git_sha,
    }
}
