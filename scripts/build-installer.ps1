$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot

try {
    if (-not (Test-Path "vendor\bin\ytdlp\yt-dlp.exe") -or
        -not (Test-Path "vendor\bin\deno\deno.exe") -or
        -not (Test-Path "vendor\bin\ffmpeg\ffmpeg.exe")) {
        Write-Host "Binários em vendor\bin\ ausentes. Executando fetch-binaries.ps1..." -ForegroundColor Cyan
        & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "fetch-binaries.ps1")
    }

    if (-not (Test-Path "vendor\bin\webview2\MicrosoftEdgeWebview2Setup.exe")) {
        Write-Host "Baixando bootstrapper oficial do Microsoft Edge WebView2..." -ForegroundColor Cyan
        New-Item -ItemType Directory -Force -Path "vendor\bin\webview2" | Out-Null
        Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile "vendor\bin\webview2\MicrosoftEdgeWebview2Setup.exe"
    }

    if (-not (Test-Path "node_modules")) {
        Write-Host "Instalando dependências do frontend (npm install)..." -ForegroundColor Cyan
        & npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install falhou" }
    }

    Write-Host "Compilando Boladão e gerando Instalador Op-Art nativo (Windows x64)..." -ForegroundColor Cyan
    & npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build --no-bundle falhou" }

    $BuiltExe = Join-Path $RepoRoot "src-tauri\target\release\boladao.exe"
    if (-not (Test-Path $BuiltExe)) {
        throw "Executável compilado não encontrado em $BuiltExe"
    }

    # 1. Atualiza o executável portátil em dist\Boladao.exe
    $DistDir = Join-Path $RepoRoot "dist"
    New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
    $FinalPortable = Join-Path $DistDir "Boladao.exe"
    try {
        Copy-Item -Force -Path $BuiltExe -Destination $FinalPortable -ErrorAction Stop
    } catch {
        Copy-Item -Force -Path $BuiltExe -Destination (Join-Path $DistDir "Boladao-novo.exe")
    }

    # 2. Gera o instalador Op-Art nativo em instalador\Boladao-Instalador.exe (com marcador de 16 bytes BOLADAO_INSTALL!)
    $InstallerDir = Join-Path $RepoRoot "instalador"
    New-Item -ItemType Directory -Force -Path $InstallerDir | Out-Null
    $FinalInstaller = Join-Path $InstallerDir "Boladao-Instalador.exe"
    Copy-Item -Force -Path $BuiltExe -Destination $FinalInstaller

    $MagicBytes = [System.Text.Encoding]::ASCII.GetBytes("BOLADAO_INSTALL!")
    $Stream = [System.IO.File]::Open($FinalInstaller, [System.IO.FileMode]::Append, [System.IO.FileAccess]::Write)
    try {
        $Stream.Write($MagicBytes, 0, $MagicBytes.Length)
    } finally {
        $Stream.Close()
    }

    $SizeMb = [Math]::Round((Get-Item $FinalInstaller).Length / 1MB, 2)
    Write-Host "`nInstalador Op-Art e versão portátil criados com sucesso!" -ForegroundColor Green
    Write-Host "  Instalador : $FinalInstaller ($SizeMb MB)"
    Write-Host "  Portátil   : $FinalPortable"
} finally {
    Pop-Location
}
