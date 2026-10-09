# Botgek sürüm yayınlama (Windows) — mantığın **tek kaynağı** `release.sh`'tir.
# Bu sarmalayıcı yalnızca Git for Windows ile gelen bash'i bulup script'i ona devreder
# (bump/etiket/push mantığı kopyalanmaz, iki yerde tutulmaz).
#
# Kullanım:
#   powershell -File release.ps1 patch|minor|major|<x.y.z> [--push] [--no-test]
#   pwsh -File release.ps1 minor --push
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)

$ErrorActionPreference = 'Stop'

$candidates = @(
    'C:\Program Files\Git\bin\bash.exe',
    'C:\Program Files\Git\usr\bin\bash.exe',
    'C:\Program Files (x86)\Git\bin\bash.exe'
)

$bash = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $bash) {
    $found = Get-Command bash.exe -ErrorAction SilentlyContinue
    if ($found) { $bash = $found.Source }
}
if (-not $bash) {
    throw "bash bulunamadı (Git for Windows gerekli). Alternatif: 'sh release.sh <args>' veya WSL."
}

$script = Join-Path $PSScriptRoot 'release.sh'
if (-not (Test-Path -LiteralPath $script)) {
    throw "release.sh bulunamadı: $script"
}

& $bash $script @Rest
exit $LASTEXITCODE
