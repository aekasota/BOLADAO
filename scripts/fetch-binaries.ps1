param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ManifestPath = Join-Path $RepoRoot "vendor\manifest.json"
$CacheDir = Join-Path $RepoRoot "vendor\cache"
$BinDir = Join-Path $RepoRoot "vendor\bin"

if (-not (Test-Path $ManifestPath)) {
    throw "Manifest not found at $ManifestPath"
}

New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null

$Manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json

function Assert-MicrosoftSignature {
    param([string]$Path)
    # Authenticode válido e assinado pela Microsoft; qualquer outra coisa não entra no instalador.
    $sig = Get-AuthenticodeSignature -FilePath $Path
    $subject = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { "" }
    if ($sig.Status -ne "Valid" -or $subject -notmatch "O=Microsoft Corporation") {
        throw "Assinatura inválida em $Path (status: $($sig.Status); assinante: $subject). O arquivo não pode ir para o instalador."
    }
}

function Verify-RemoteChecksum {
    param(
        [string]$ChecksumUrl,
        [string]$AssetName,
        [string]$ExpectedHash
    )
    if (-not $ChecksumUrl) { return }
    Write-Host "  -> Conferindo arquivo de checksum publicado ($ChecksumUrl)..." -ForegroundColor DarkGray
    $resp = Invoke-WebRequest -Uri $ChecksumUrl -UseBasicParsing -Headers @{ "User-Agent" = "Boladao-Build" }
    $text = [System.Text.Encoding]::UTF8.GetString($resp.Content)
    if ($text.ToLowerInvariant() -notmatch $ExpectedHash.ToLowerInvariant()) {
        throw "Published checksum file at $ChecksumUrl does not contain expected hash $ExpectedHash for $AssetName!"
    }
}

function Download-And-Verify {
    param(
        [pscustomobject]$Entry
    )
    $archivePath = Join-Path $CacheDir "$($Entry.name)-$($Entry.version)-$($Entry.asset)"
    $needDownload = $Force -or (-not (Test-Path $archivePath))

    if (-not $needDownload) {
        $existingHash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($existingHash -ne $Entry.sha256.ToLowerInvariant()) {
            Write-Host "  -> Hash diferente no cache para $($Entry.name), baixando novamente..." -ForegroundColor Yellow
            $needDownload = $true
        }
    }

    if ($needDownload) {
        if ($Entry.checksum_url) {
            Verify-RemoteChecksum -ChecksumUrl $Entry.checksum_url -AssetName $Entry.asset -ExpectedHash $Entry.sha256
        }
        Write-Host "Baixando $($Entry.name) $($Entry.version) ($($Entry.url))..." -ForegroundColor Cyan
        $tmpPath = "$archivePath.tmp"
        if (Test-Path $tmpPath) { Remove-Item -Force $tmpPath }
        & curl.exe -fL --retry 3 --retry-delay 2 -o $tmpPath $Entry.url
        if ($LASTEXITCODE -ne 0) {
            throw "Download falhou para $($Entry.url)"
        }
        $actualHash = (Get-FileHash -Path $tmpPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne $Entry.sha256.ToLowerInvariant()) {
            Remove-Item -Force $tmpPath -ErrorAction SilentlyContinue
            throw "SHA-256 inválido para $($Entry.name)! Esperado: $($Entry.sha256), Obtido: $actualHash"
        }
        Move-Item -Force $tmpPath $archivePath
    } else {
        Write-Host "Cache válido para $($Entry.name) $($Entry.version) (SHA-256 OK)." -ForegroundColor Green
    }

    return $archivePath
}

# 1. yt-dlp (unpackaged directory build)
$ytdlpZip = Download-And-Verify -Entry $Manifest.ytdlp
$ytdlpDest = Join-Path $BinDir "ytdlp"
$ytdlpTmp = Join-Path $BinDir "ytdlp_tmp"
if (Test-Path $ytdlpTmp) { Remove-Item -Recurse -Force $ytdlpTmp }
if (Test-Path $ytdlpDest) { Remove-Item -Recurse -Force $ytdlpDest }
New-Item -ItemType Directory -Force -Path $ytdlpTmp | Out-Null
Expand-Archive -Path $ytdlpZip -DestinationPath $ytdlpTmp -Force

$ytdlpExe = Get-ChildItem -Path $ytdlpTmp -Filter "yt-dlp.exe" -Recurse | Select-Object -First 1
if (-not $ytdlpExe) { throw "yt-dlp.exe não encontrado dentro de $ytdlpZip" }
Move-Item -Path $ytdlpExe.Directory.FullName -Destination $ytdlpDest
if (Test-Path $ytdlpTmp) { Remove-Item -Recurse -Force $ytdlpTmp }
Set-Content -Path (Join-Path $ytdlpDest ".version") -Value $Manifest.ytdlp.version -NoNewline -Encoding UTF8

# 2. Deno
$denoZip = Download-And-Verify -Entry $Manifest.deno
$denoDest = Join-Path $BinDir "deno"
$denoTmp = Join-Path $BinDir "deno_tmp"
if (Test-Path $denoTmp) { Remove-Item -Recurse -Force $denoTmp }
if (Test-Path $denoDest) { Remove-Item -Recurse -Force $denoDest }
New-Item -ItemType Directory -Force -Path $denoTmp | Out-Null
Expand-Archive -Path $denoZip -DestinationPath $denoTmp -Force

$denoExe = Get-ChildItem -Path $denoTmp -Filter "deno.exe" -Recurse | Select-Object -First 1
if (-not $denoExe) { throw "deno.exe não encontrado dentro de $denoZip" }
New-Item -ItemType Directory -Force -Path $denoDest | Out-Null
Move-Item -Path $denoExe.FullName -Destination (Join-Path $denoDest "deno.exe")
Remove-Item -Recurse -Force $denoTmp
Set-Content -Path (Join-Path $denoDest ".version") -Value $Manifest.deno.version -NoNewline -Encoding UTF8

# 3. FFmpeg + FFprobe (full shared build)
$ffZip = Download-And-Verify -Entry $Manifest.ffmpeg
$ffDest = Join-Path $BinDir "ffmpeg"
$ffTmp = Join-Path $BinDir "ffmpeg_tmp"
if (Test-Path $ffTmp) { Remove-Item -Recurse -Force $ffTmp }
if (Test-Path $ffDest) { Remove-Item -Recurse -Force $ffDest }
New-Item -ItemType Directory -Force -Path $ffTmp | Out-Null
Expand-Archive -Path $ffZip -DestinationPath $ffTmp -Force

$ffExe = Get-ChildItem -Path $ffTmp -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1
if (-not $ffExe) { throw "ffmpeg.exe não encontrado dentro de $ffZip" }
New-Item -ItemType Directory -Force -Path $ffDest | Out-Null

# Copy ffmpeg.exe, ffprobe.exe, and all shared DLLs (exclude ffplay.exe which is unused GUI player)
Get-ChildItem -Path $ffExe.Directory.FullName -File | Where-Object {
    $_.Name -in @("ffmpeg.exe", "ffprobe.exe") -or $_.Extension -eq ".dll"
} | ForEach-Object {
    Move-Item -Path $_.FullName -Destination (Join-Path $ffDest $_.Name)
}
# Copy LICENSE.txt if present in parent dir
$ffRoot = $ffExe.Directory.Parent
if ($ffRoot -and (Test-Path (Join-Path $ffRoot.FullName "LICENSE"))) {
    Copy-Item (Join-Path $ffRoot.FullName "LICENSE") -Destination (Join-Path $ffDest "LICENSE")
}
Remove-Item -Recurse -Force $ffTmp
Set-Content -Path (Join-Path $ffDest ".version") -Value $Manifest.ffmpeg.version -NoNewline -Encoding UTF8

# 4. Microsoft Edge WebView2 Bootstrapper (embutido em src-tauri/src/installer/mod.rs)
$wv2Dir = Join-Path $BinDir "webview2"
$wv2Exe = Join-Path $wv2Dir "MicrosoftEdgeWebview2Setup.exe"
if ($Force -or (-not (Test-Path $wv2Exe))) {
    Write-Host "Baixando bootstrapper oficial do Microsoft Edge WebView2..." -ForegroundColor Cyan
    New-Item -ItemType Directory -Force -Path $wv2Dir | Out-Null
    # Termina em .exe: o Windows escolhe o verificador de assinatura pela extensão.
    $wv2Tmp = Join-Path $wv2Dir "MicrosoftEdgeWebview2Setup.download.exe"
    if (Test-Path $wv2Tmp) { Remove-Item -Force $wv2Tmp }
    & curl.exe -fL --retry 3 --retry-delay 2 -o $wv2Tmp "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
    if ($LASTEXITCODE -ne 0) {
        throw "Download falhou para o bootstrapper do Microsoft Edge WebView2"
    }
    try {
        Assert-MicrosoftSignature $wv2Tmp
    } catch {
        Remove-Item -Force $wv2Tmp
        throw
    }
    Move-Item -Force $wv2Tmp $wv2Exe
} else {
    Assert-MicrosoftSignature $wv2Exe
    Write-Host "Cache válido para MicrosoftEdgeWebview2Setup.exe." -ForegroundColor Green
}

Write-Host "`nBinários prontos em vendor\bin\:" -ForegroundColor Green
Write-Host "  yt-dlp  : $(& (Join-Path $ytdlpDest 'yt-dlp.exe') --version)"
Write-Host "  deno    : $((& (Join-Path $denoDest 'deno.exe') --version)[0])"
Write-Host "  ffmpeg  : $((& (Join-Path $ffDest 'ffmpeg.exe') -version)[0])"
Write-Host "  webview2: $wv2Exe"
