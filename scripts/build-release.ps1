$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot

try {
    if (-not (Test-Path "vendor\bin\ytdlp\yt-dlp.exe") -or
        -not (Test-Path "vendor\bin\deno\deno.exe") -or
        -not (Test-Path "vendor\bin\ffmpeg\ffmpeg.exe") -or
        -not (Test-Path "vendor\bin\webview2\MicrosoftEdgeWebview2Setup.exe")) {
        Write-Host "Binários em vendor\bin\ ausentes. Executando fetch-binaries.ps1..." -ForegroundColor Cyan
        & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "fetch-binaries.ps1")
    }

    if (-not (Test-Path "node_modules")) {
        Write-Host "Instalando dependências do frontend (npm install)..." -ForegroundColor Cyan
        & npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install falhou" }
    }

    Write-Host "Compilando Boladão em modo release portátil (--no-bundle)..." -ForegroundColor Cyan
    & npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build --no-bundle falhou" }

    $BuiltExe = Join-Path $RepoRoot "src-tauri\target\release\boladao.exe"
    if (-not (Test-Path $BuiltExe)) {
        throw "Executável compilado não encontrado em $BuiltExe"
    }

    $OutDir = Join-Path $RepoRoot "dist"
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $FinalExe = Join-Path $OutDir "Boladao.exe"
    try {
        Copy-Item -Force -Path $BuiltExe -Destination $FinalExe -ErrorAction Stop
        $TargetShown = $FinalExe
    } catch {
        $AltExe = Join-Path $OutDir "Boladao-novo.exe"
        Copy-Item -Force -Path $BuiltExe -Destination $AltExe
        Write-Host "`nAviso: dist\Boladao.exe está aberto no momento; cópia salva em dist\Boladao-novo.exe (e em src-tauri\target\release\boladao.exe)." -ForegroundColor Yellow
        $TargetShown = $AltExe
    }

    $SizeMb = [Math]::Round((Get-Item $TargetShown).Length / 1MB, 2)
    Write-Host "`nBuild concluída com sucesso!" -ForegroundColor Green
    Write-Host "  Arquivo : $TargetShown"
    Write-Host "  Tamanho : $SizeMb MB"
} finally {
    Pop-Location
}
