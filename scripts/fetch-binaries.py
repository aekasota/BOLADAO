#!/usr/bin/env python3
"""Prepara os motores (yt-dlp, Deno e FFmpeg) de um alvo Linux ou macOS em vendor/bin/<alvo>.

Cada arquivo baixado é conferido pelo SHA-256 de vendor/<alvo>.json antes de ser usado. No macOS,
o FFmpeg é compilado a partir do código-fonte (scripts/build-ffmpeg-macos.sh). O Windows continua
com scripts/fetch-binaries.ps1.

Uso: python3 scripts/fetch-binaries.py [alvo]   (padrão: o alvo do rustc desta máquina)
"""
import argparse
import hashlib
import json
import platform
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor"
CACHE = VENDOR / "cache"
SUPPORTED = ("x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-apple-darwin")


def host_target():
    try:
        out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    match = re.search(r"^host: (\S+)$", out, re.M)
    return match.group(1) if match else None


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url, dest):
    request = urllib.request.Request(url, headers={"User-Agent": "Boladao-Build"})
    with urllib.request.urlopen(request, timeout=120) as response, open(dest, "wb") as out:
        shutil.copyfileobj(response, out, 1 << 20)


def verified(entry):
    """Returns the cached file for `entry`, downloading it when missing or with another hash."""
    expected = entry["sha256"].lower()
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / f"{expected}-{entry['asset']}"
    if cached.is_file() and sha256_of(cached) == expected:
        return cached
    if entry.get("checksum_url"):
        request = urllib.request.Request(entry["checksum_url"], headers={"User-Agent": "Boladao-Build"})
        with urllib.request.urlopen(request, timeout=60) as response:
            published = response.read().decode("utf-8", "replace").lower()
        if expected not in published:
            sys.exit(f"O checksum publicado em {entry['checksum_url']} não confere com o manifesto.")
    print(f"Baixando {entry['name']} {entry['version']} ({entry['url']})...")
    partial = cached.with_suffix(cached.suffix + ".part")
    download(entry["url"], partial)
    actual = sha256_of(partial)
    if actual != expected:
        partial.unlink()
        sys.exit(f"SHA-256 inválido para {entry['url']}: esperado {expected}, obtido {actual}.")
    partial.replace(cached)
    return cached


def make_executable(path):
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def extract_zip(archive_path, dest):
    """Extracts a zip keeping the Unix permissions stored in it (zipfile alone drops them)."""
    with zipfile.ZipFile(archive_path) as archive:
        for info in archive.infolist():
            path = Path(archive.extract(info, dest))
            mode = (info.external_attr >> 16) & 0o777
            if mode and path.is_file():
                path.chmod(mode)


def stage_ytdlp(entry, component):
    asset = verified(entry)
    if entry["asset"].endswith(".zip"):
        # Build "onedir": o executável vem ao lado da pasta _internal e passa a se chamar yt-dlp.
        extract_zip(asset, component)
        exe = next(p for p in component.rglob("yt-dlp*") if p.is_file() and p.suffix == "")
        if exe.parent != component:
            for item in exe.parent.iterdir():
                item.rename(component / item.name)
            exe = component / exe.name
        exe.rename(component / "yt-dlp")
    else:
        shutil.copyfile(asset, component / "yt-dlp")
    make_executable(component / "yt-dlp")


def stage_deno(entry, component):
    with zipfile.ZipFile(verified(entry)) as archive, archive.open("deno") as src:
        with open(component / "deno", "wb") as out:
            shutil.copyfileobj(src, out)
    make_executable(component / "deno")


SONAME = re.compile(r"^lib.+\.so\.\d+$")


def stage_ffmpeg_linux(entry, component, stage):
    with tarfile.open(verified(entry)) as archive:
        archive.extractall(stage, filter="data")
    ffmpeg = next(stage.rglob("bin/ffmpeg"))
    top = ffmpeg.parent.parent
    (component / "bin").mkdir()
    (component / "lib").mkdir()
    for name in ("ffmpeg", "ffprobe"):
        shutil.copyfile(top / "bin" / name, component / "bin" / name)
        make_executable(component / "bin" / name)
    # Só os nomes que o carregador procura (SONAME, como libavcodec.so.62), com o conteúdo real:
    # o empacotamento do build.rs guarda arquivos, não links simbólicos.
    copied = 0
    for lib in sorted((top / "lib").iterdir()):
        if SONAME.match(lib.name):
            shutil.copyfile(lib.resolve(), component / "lib" / lib.name)
            copied += 1
    if copied == 0:
        sys.exit("Nenhuma biblioteca compartilhada encontrada no pacote do FFmpeg.")
    license_file = next((p for p in (top / "LICENSE.txt", top / "LICENSE") if p.is_file()), None)
    if license_file:
        shutil.copyfile(license_file, component / "LICENSE")


def stage_ffmpeg_macos(target, component, stage):
    if platform.system() != "Darwin":
        sys.exit("O FFmpeg do macOS é compilado a partir do código-fonte e precisa de um Mac (ou do CI).")
    sources = stage / "sources"
    sources.mkdir()
    for name, src in json.loads((VENDOR / "ffmpeg-sources.json").read_text()).items():
        entry = {"name": name, "version": "", "asset": src["url"].rsplit("/", 1)[-1], **src}
        (sources / name).symlink_to(verified(entry))
    # Through bash: a checkout made on Windows does not keep the script's executable bit.
    subprocess.run(["bash", str(ROOT / "scripts" / "build-ffmpeg-macos.sh"), target, str(sources), str(component)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("target", nargs="?", default=None)
    target = parser.parse_args().target or host_target()
    if not target:
        sys.exit("Informe o alvo (por exemplo, x86_64-unknown-linux-gnu): o rustc não foi encontrado.")
    if target not in SUPPORTED:
        sys.exit(f"Alvo sem manifesto: {target}. No Windows, use scripts/fetch-binaries.ps1.")
    manifest = json.loads((VENDOR / f"{target}.json").read_text())
    destination = VENDOR / "bin" / target
    destination.mkdir(parents=True, exist_ok=True)

    for key in ("ytdlp", "deno", "ffmpeg"):
        entry = manifest[key]
        installed = destination / key
        stamp = f"{entry['version']}:{entry['sha256']}"
        if (installed / ".source").is_file() and (installed / ".source").read_text().strip() == stamp:
            print(f"{key} {entry['version']}: já preparado.")
            continue
        with tempfile.TemporaryDirectory(dir=destination) as tmp:
            stage = Path(tmp)
            component = stage / key
            component.mkdir()
            if key == "ytdlp":
                stage_ytdlp(entry, component)
            elif key == "deno":
                stage_deno(entry, component)
            elif entry.get("build") == "source":
                stage_ffmpeg_macos(target, component, stage)
            else:
                stage_ffmpeg_linux(entry, component, stage / "extract")
            (component / ".version").write_text(entry["version"])
            (component / ".source").write_text(stamp)
            if installed.exists():
                shutil.rmtree(installed)
            component.rename(installed)
        print(f"{key} {entry['version']}: pronto em {installed}.")

    if target == host_target():
        ff = destination / "ffmpeg" / ("bin/ffmpeg" if "linux" in target else "ffmpeg")
        for cmd in ([destination / "ytdlp" / "yt-dlp", "--version"],
                    [destination / "deno" / "deno", "--version"],
                    [ff, "-version"]):
            first = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, check=True).stdout.splitlines()[0]
            print(f"  {cmd[0].name}: {first}")


if __name__ == "__main__":
    main()
