<#
.SYNOPSIS
Botgek ayar yedeği (Windows) — **göç öncesi** ayar dosyasının zaman damgalı kopyasını alır.

.DESCRIPTION
Kullanım: `powershell -File backup-settings.ps1 [-Path <settings.json yolu>]` (PowerShell 7+ kuruluysa `pwsh -File ...` ile de çalışır).

Yol verilmezse botun kendi çözüm sırası izlenir: `BOTGEK_SETTINGS_PATH`, `BOTGEK_DB_PATH` bir dizin içeriyorsa o dizindeki `settings.json`, ardından `settings.json` ve `settings.json\settings.json`.

Docker Compose kurulumunda dosya volume içindedir: önce `docker compose exec botgek cat /app/data/settings.json > settings.docker.json`, sonra `-Path settings.docker.json` (hosttaki `settings.json` ilk içe aktarım kaynağıdır, üzerine yazmayın).
systemd/`.deb` kurulumunda dosya `/var/lib/botgek/settings.json`'dır; orada `backup-settings.sh` kullanın.

Yedek adı `settings.json.bak-yyyyMMdd-HHmmss`; ad varsa `-1`, `-2`… eklenir, mevcut yedeğin üzerine asla yazılmaz.
Kopya önce aynı dizinde geçici dosyaya alınır, doğrulanır ve üzerine yazmayan bir taşımayla yerine konur.

Geri alma: yedeği `settings.json` üzerine kopyalayın ve botu **eski sürümle** yeniden başlatın (eski sürüm tam kopya sunucu kayıtlarını bekler).
#>
param([string]$Path = '')

$ErrorActionPreference = 'Stop'

if (-not $Path -and $env:BOTGEK_SETTINGS_PATH) {
    $Path = $env:BOTGEK_SETTINGS_PATH
}

if (-not $Path -and $env:BOTGEK_DB_PATH -and $env:BOTGEK_DB_PATH -ne ':memory:') {
    $dbDir = Split-Path -Parent $env:BOTGEK_DB_PATH
    if ($dbDir) {
        $candidate = Join-Path $dbDir 'settings.json'
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Path = $candidate }
    }
}

if (-not $Path) {
    foreach ($candidate in @('settings.json', 'settings.json\settings.json')) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Path = $candidate; break }
    }
}

if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw 'ayar dosyası bulunamadı (yolu -Path ile verin; Docker için: docker compose exec botgek cat /app/data/settings.json > settings.docker.json)'
}

$Path = (Resolve-Path -LiteralPath $Path).ProviderPath

try {
    Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json | Out-Null
} catch {
    Write-Warning "'$Path' geçerli JSON olarak okunamadı (yine de yedekleniyor)"
}

$dir = Split-Path -Parent $Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$temp = Join-Path $dir ('.{0}.bak-tmp.{1}' -f (Split-Path -Leaf $Path), [guid]::NewGuid().ToString('N'))
$backup = $null

try {
    Copy-Item -LiteralPath $Path -Destination $temp
    if ((Get-FileHash -LiteralPath $Path).Hash -ne (Get-FileHash -LiteralPath $temp).Hash) {
        throw "geçici kopya doğrulanamadı ($temp)"
    }

    # `File.Move` hedef varsa hata verir: ad seçimi ile yerleştirme tek adımdır, mevcut yedek ezilmez.
    for ($n = 0; $n -le 100; $n++) {
        $name = if ($n -eq 0) { '{0}.bak-{1}' -f $Path, $stamp } else { '{0}.bak-{1}-{2}' -f $Path, $stamp, $n }
        try {
            [System.IO.File]::Move($temp, $name)
            $backup = $name
            break
        } catch [System.IO.IOException] {
            if (-not (Test-Path -LiteralPath $name)) { throw }
        }
    }
    if (-not $backup) {
        throw "'$Path.bak-$stamp' için boş yedek adı bulunamadı"
    }
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Force }
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
