# Самопроверка: собирает программу, прогоняет модульные тесты и сценарий с настоящим
# вводом, печатает отчёт. Снимки и отчёт остаются в папке selftest-out.
#   .\selftest.ps1            отладочная сборка
#   .\selftest.ps1 -Release   выпускная сборка
param([switch]$Release)

$root = $PSScriptRoot
$out = Join-Path $root "selftest-out"
$profile = if ($Release) { "release" } else { "debug" }

Push-Location $root
try {
    cargo test --quiet
    if ($LASTEXITCODE -ne 0) { exit 1 }
    if ($Release) { cargo build --release } else { cargo build }
    if ($LASTEXITCODE -ne 0) { exit 1 }
} finally { Pop-Location }

Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
$exe = Join-Path $root "target\$profile\kartograf.exe"
$p = Start-Process -FilePath $exe -ArgumentList "--selftest", "`"$out`"" -PassThru
if (-not $p.WaitForExit(120000)) { $p.Kill(); Write-Output "ЗАВИСЛО: сценарий не закончился за две минуты"; exit 2 }

$report = Join-Path $out "report.txt"
if (Test-Path $report) { Get-Content $report -Encoding UTF8 } else { Write-Output "отчёта нет" }
exit $p.ExitCode
