# Круг сверки местности из OSM со спутником: каждый участок выгружается с паками FA,
# склеивается с кронами (как в Foundry) и кладётся рядом со снимком по <bounds>.
#
#   powershell -File tools\osm\round.ps1 -Areas malham,lulworth -Tag r11
#
# Участки — `<имя>.osm` в «Документы\Картограф OSM». Там же `fa_data` (настройки с
# путями к пакам, отдельные от настроек редактора) и кеш снимков `sat_tiles`.
# Результат — в `круги\<Tag>_<имя>.jpg` (карта) и `<Tag>_cmp_<имя>.jpg` (рядом со снимком).
# Перед кругом пересобрать exe: `cargo build --release` (`cargo test` его не обновляет).
param(
    [string[]]$Areas = @('bolton_abbey', 'tarn_hows', 'tarn_hows_lake', 'lulworth', 'malham'),
    [int]$Px = 24,
    [string]$Tag = 'r',
    [string]$Osm = (Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Картограф OSM'),
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\kartograf.exe')
)
# Через -File список приходит одной строкой «a,b,c» — разбить.
$Areas = $Areas | ForEach-Object { $_ -split ',' } | Where-Object { $_ }
$here = $PSScriptRoot
$work = Join-Path $Osm 'круги'
New-Item -ItemType Directory -Force $work | Out-Null
foreach ($a in $Areas) {
    $src = Join-Path $Osm "$a.osm"
    if (-not (Test-Path $src)) { "$a : нет $src"; continue }
    $dir = Join-Path $work "${Tag}_$a"
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $dir | Out-Null
    $t = Get-Date
    # Пути с пробелами — в кавычках, иначе exe получает обрывок и выгружает пустую карту.
    $p = Start-Process $Exe -ArgumentList @('--osm', "`"$src`"", '--export', "`"$dir`"", '--px', "$Px", '--data-dir', "`"$(Join-Path $Osm 'fa_data')`"", '--aside') -PassThru
    if (-not $p.WaitForExit(400000)) { $p.Kill(); "$a : не уложился в 400 с"; continue }
    $j = Get-Content (Join-Path $dir 'Новая карта.json') -Raw -Encoding utf8 | ConvertFrom-Json
    "{0}: {1:N0} с, тайлов {2}" -f $a, ((Get-Date) - $t).TotalSeconds, $j.tiles.Count
    $map = Join-Path $work "${Tag}_$a.jpg"
    python -I (Join-Path $here 'compose.py') $dir $map (1600.0 / $j.width).ToString([Globalization.CultureInfo]::InvariantCulture)
    python -I (Join-Path $here 'sat.py') $src $map (Join-Path $work "${Tag}_cmp_$a.jpg")
}
