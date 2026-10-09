# Botgek ayar yedeği (Windows) — **göç öncesi** ayar dosyasının zaman damgalı kopyasını alır.
#
# Kullanım:
#   powershell -File backup-settings.ps1 [-Path <settings.json yolu>]
#   (PowerShell 7+ kuruluysa aynı komut `pwsh -File ...` ile de çalışır.)
#
# Yol verilmezse sırayla `settings.json` ve `settings.json\settings.json` denenir.
# Yedek adı: `settings.json.bak-yyyyMMdd-HHmmss`
#
# Geri alma: yedeği `settings.json` üzerine kopyalayın ve botu **eski sürümle** yeniden
# başlatın (eski sürüm tam kopya sunucu kayıtlarını bekler).
param([string]$Path = '')

$ErrorActionPreference = 'Stop'

if (-not $Path) {
    foreach ($candidate in @('settings.json', 'settings.json\settings.json')) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Path = $candidate; break }
    }
}

if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw 'ayar dosyası bulunamadı (yolu -Path ile verin)'
}

try {
    Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json | Out-Null
} catch {
    Write-Warning "'$Path' geçerli JSON olarak okunamadı (yine de yedekleniyor)"
}

$backup = '{0}.bak-{1}' -f $Path, (Get-Date -Format 'yyyyMMdd-HHmmss')
Copy-Item -LiteralPath $Path -Destination $backup -Force

if ((Get-FileHash -LiteralPath $Path).Hash -ne (Get-FileHash -LiteralPath $backup).Hash) {
    throw "yedek doğrulanamadı ($backup)"
}

# Göç öncesi durum fotoğrafı: şema sürümü ve kayıtların hangi bölümleri tuttuğu.
$data = Get-Content -LiteralPath $backup -Raw -Encoding UTF8 | ConvertFrom-Json
$guilds = $data.guilds
$count = 0
if ($guilds) { $count = @($guilds.PSObject.Properties).Count }
Write-Host ("şema sürümü: {0} · sunucu kaydı: {1}" -f $(if ($null -ne $data.schema_version) { $data.schema_version } else { 0 }), $count)
if ($guilds) {
    foreach ($prop in $guilds.PSObject.Properties) {
        $pinned = @('modules', 'role_filters', 'ai', 'engine') | Where-Object { $null -ne $prop.Value.$_ }
        $label = if ($pinned.Count) { $pinned -join ', ' } else { '—' }
        Write-Host ("  {0}: {1}" -f $prop.Name, $label)
    }
}

Write-Host "yedek alındı: $backup"
