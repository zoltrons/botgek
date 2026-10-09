// Derleme zamanı sürüm bilgisi (`cargo:rustc-env`).
//
// **Tek kaynak:** sürüm türetme mantığı `build-support/version-parse.rs` içindedir ve aynı
// dosya `src/version.rs` tarafından da `include!` edilir. Böylece gömülen sürüm ile çalışma
// zamanında raporlanan sürüm ayrışamaz (`src/version.rs` testi bunu doğrular).
//
// Girdi önceliği:
//   1. `BOTGEK_BUILD_VERSION` — CI/Docker tag derlemesi (ör. `v0.2.0`).
//   2. `git describe`         — yerel ağaç: commit sayısı + kısa sha + `dirty`.
//   3. `Cargo.toml` sürümü    — git yoksa (tarball/Docker bağlamı) güvenli taban.
//
// Sürüm senkronu denetlenir: etiket/override `Cargo.toml` ile ayrışırsa derleme **uyarı**
// verir (sessiz sapma olmaz). Bu dosya dış bağımlılık kullanmaz.
include!("build-support/version-parse.rs");

use std::env;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let target = env::var("TARGET").or_else(|_| env::var("HOST")).unwrap_or_else(|_| "bilinmeyen".to_string());

    let override_version = non_empty_env("BOTGEK_BUILD_VERSION");
    let describe = git(&manifest_dir, &[
        "describe", "--tags", "--long", "--dirty", "--always", "--match", "v[0-9]*",
    ]);
    let git_sha = non_empty_env("BOTGEK_GIT_SHA")
        .map(|value| value.chars().take(7).collect::<String>())
        .or_else(|| git(&manifest_dir, &["rev-parse", "--short", "HEAD"]));

    let derived = derive_version(&RawVersionInputs {
        pkg_version: &pkg_version,
        override_version: override_version.as_deref(),
        describe: describe.as_deref(),
        git_sha: git_sha.as_deref(),
    });

    let package_semver = normalize_semver(&pkg_version);

    // Sürüm senkronu: tag/override ile `Cargo.toml` ayrışırsa uyar (release script'i ikisini
    // birlikte günceller; elle tag atmak sürümü sessizce kaydırabilirdi).
    match (&override_version, &derived) {
        (Some(raw), _) if normalize_semver(raw) != package_semver => println!(
            "cargo:warning=BOTGEK_BUILD_VERSION ({}) ile Cargo.toml ({}) ayrışık — sürüm senkron değil",
            derived.semver, pkg_version
        ),
        (None, d) if !d.semver.is_empty() && d.semver != package_semver => println!(
            "cargo:warning=en yakın etiket ({}) ile Cargo.toml ({}) ayrışık — release script'ini kullanın",
            derived.semver, pkg_version
        ),
        _ => {}
    }

    let build_date = non_empty_env("BOTGEK_BUILD_DATE").unwrap_or_else(|| {
        let seconds = non_empty_env("SOURCE_DATE_EPOCH")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs() as i64)
                    .unwrap_or_default()
            });
        format_unix_utc(seconds)
    });

    emit("BOTGEK_VERSION_SEMVER", &derived.semver);
    emit("BOTGEK_VERSION_DISPLAY", &derived.display);
    emit("BOTGEK_VERSION_FULL", &derived.full);
    emit("BOTGEK_GIT_SHA", derived.git_sha.as_deref().unwrap_or("unknown"));
    emit("BOTGEK_GIT_TAG", derived.tag.as_deref().unwrap_or(""));
    emit(
        "BOTGEK_COMMITS_SINCE_TAG",
        &derived.commits_since_tag.map(|count| count.to_string()).unwrap_or_default(),
    );
    emit("BOTGEK_IS_RELEASE", if derived.is_release { "true" } else { "false" });
    emit("BOTGEK_IS_DIRTY", if derived.is_dirty { "true" } else { "false" });
    emit("BOTGEK_BUILD_DATE", &build_date);
    emit("BOTGEK_TARGET", &target);
    emit("BOTGEK_USER_AGENT", &format!("Botgek/{} (Discord link converter)", derived.display));
    emit(
        "BOTGEK_DISCORD_USER_AGENT",
        &format!("DiscordBot (https://github.com/zoltrons/botgek, {})", derived.semver),
    );

    // Ham girdiler: çalışma zamanı aynı fonksiyonla yeniden çözebilsin (teşhis + test).
    emit("BOTGEK_VERSION_OVERRIDE", override_version.as_deref().unwrap_or(""));
    emit("BOTGEK_GIT_DESCRIBE", describe.as_deref().unwrap_or(""));

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build-support/version-parse.rs");
    // `.git` yoksa (Docker bağlamı) yönergeleri yazmayız: aksi hâlde build script her derlemede
    // gereksiz yeniden çalışır ve önbellek etkisizleşir.
    if Path::new(&manifest_dir).join(".git").exists() {
        for path in [".git/HEAD", ".git/index", ".git/refs/tags", ".git/packed-refs"] {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    for key in ["BOTGEK_BUILD_VERSION", "BOTGEK_GIT_SHA", "BOTGEK_BUILD_DATE", "SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
}

/// Boş/whitespace değerleri `None` sayan ortam okuması.
fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Git komutunu çalıştırır; başarısızsa (git yok, depo yok) `None` döner.
fn git(dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Değeri crate'e gömülecek ortam değişkeni olarak yazdırır (satır sonu içermez).
fn emit(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}
