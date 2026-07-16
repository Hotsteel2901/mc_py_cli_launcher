#!/usr/bin/env python3

import os
import re
import sys
import json
import time
import shutil
import socket
import hashlib
import zipfile
import platform
import argparse
import webbrowser
import threading
import subprocess
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse, parse_qs, urlencode
from urllib.request import Request, urlopen

if sys.stdout and hasattr(sys.stdout, "reconfigure") and sys.stdout.encoding.lower() != "utf-8":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass

MS_CLIENT_ID   = "00000000402b5328"
MS_REDIRECT    = "https://login.live.com/oauth20_desktop.srf"
MS_SCOPE       = "service::user.auth.xboxlive.com::MBI_SSL"
MS_AUTH_URL    = "https://login.live.com/oauth20_authorize.srf"
MS_TOKEN_URL   = "https://login.live.com/oauth20_token.srf"
MS_DEVICE_AUTH = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode"
MS_DEVICE_TOKEN = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"

XBL_AUTH_URL   = "https://user.auth.xboxlive.com/user/authenticate"
XSTS_AUTH_URL  = "https://xsts.auth.xboxlive.com/xsts/authorize"

MC_LOGIN_URL   = "https://api.minecraftservices.com/authentication/login_with_xbox"
MC_PROFILE_URL = "https://api.minecraftservices.com/minecraft/profile"
MC_STORE_URL   = "https://api.minecraftservices.com/entitlements/mcstore"

MC_MANIFEST    = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"

DEFAULT_DIR   = Path(__file__).resolve().parent / "minecraft"
ACCOUNTS_FILE = "launcher_accounts.json"
LAUNCHER_NAME = "simple-mc-cli"
LAUNCHER_VER  = "2.0.0"

MODRINTH_API  = "https://api.modrinth.com/v2"
FORGE_MAVEN = "https://maven.minecraftforge.net/net/minecraftforge/forge"
NEOFORGE_MAVEN = "https://maven.neoforged.net/releases/net/neoforged/neoforge"
FABRIC_PROJECT_ID = "P7dR8mSH"

class _Log:
    _color_ok = None
    verbose = False
    quiet = False

    RESET  = '\033[0m'
    RED    = '\033[91m'
    GREEN  = '\033[92m'
    YELLOW = '\033[93m'
    BLUE   = '\033[94m'
    CYAN   = '\033[96m'
    GRAY   = '\033[90m'
    BOLD   = '\033[1m'

    @classmethod
    def _colors(cls) -> bool:
        if cls._color_ok is not None:
            return cls._color_ok
        if os.environ.get('NO_COLOR'):
            cls._color_ok = False
        elif platform.system() == 'Windows':
            try:
                import ctypes
                k = ctypes.windll.kernel32
                k.SetConsoleMode(k.GetStdHandle(-11), 7)
                cls._color_ok = True
            except Exception:
                cls._color_ok = False
        else:
            cls._color_ok = hasattr(sys.stdout, 'isatty') and sys.stdout.isatty()
        return cls._color_ok

    @classmethod
    def _c(cls, color: str, text: str) -> str:
        return f"{color}{text}{cls.RESET}" if cls._colors() else text

    @classmethod
    def info(cls, msg: str):
        if not cls.quiet:
            print(f"  {msg}")

    @classmethod
    def success(cls, msg: str):
        if not cls.quiet:
            print(f"  {cls._c(cls.GREEN, '[OK]')} {msg}")

    @classmethod
    def warn(cls, msg: str):
        print(f"  {cls._c(cls.YELLOW, '[WARN]')} {msg}")

    @classmethod
    def error(cls, msg: str):
        print(f"  {cls._c(cls.RED, '[ERROR]')} {msg}", file=sys.stderr)

    @classmethod
    def die(cls, msg: str, hint: str = ""):
        cls.error(msg)
        if hint:
            print(f"         {cls._c(cls.GRAY, hint)}", file=sys.stderr)
        sys.exit(1)

    @classmethod
    def debug(cls, msg: str):
        if cls.verbose:
            print(f"  {cls._c(cls.GRAY, '[DBG]')} {msg}")

    @classmethod
    def header(cls, title: str):
        if not cls.quiet:
            print(f"\n  {cls._c(cls.BOLD, f'=== {title} ===')}") 
            print()

    @classmethod
    def step(cls, n: int, total: int, msg: str):
        if not cls.quiet:
            print(f"{cls._c(cls.CYAN, f'[{n}/{total}]')} {msg}")

log = _Log()

def _sha1_file(path: Path) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()

def _http_request(method: str, url: str, data=None, json_data=None,
                  headers: dict = None, timeout: int = 30,
                  max_retries: int = 3) -> tuple:

    head = (headers or {}).copy()
    head.setdefault("User-Agent", f"{LAUNCHER_NAME}/{LAUNCHER_VER}")
    body = None
    if json_data is not None:
        body = json.dumps(json_data).encode("utf-8")
        head.setdefault("Content-Type", "application/json")
    elif data is not None:
        body = data.encode("utf-8") if isinstance(data, str) else data
        head.setdefault("Content-Type", "application/x-www-form-urlencoded")

    last_error = None
    last_error_body = b""
    for attempt in range(max_retries):
        try:
            req = Request(url, data=body, headers=head, method=method.upper())
            with urlopen(req, timeout=timeout) as resp:
                return resp.status, resp.read()
        except HTTPError as e:
            body_bytes = e.read() if hasattr(e, 'read') else b""
            if 400 <= e.code < 500:
                return e.code, body_bytes
            last_error = e
            last_error_body = body_bytes
        except URLError as e:
            last_error = e
        except Exception as e:
            last_error = e

        if attempt < max_retries - 1:
            delay = 2 ** attempt
            reason = type(last_error).__name__
            log.warn(f"[retry {attempt+1}/{max_retries-1}] "
                     f"{url.rsplit('/', 1)[-1]} failed ({reason}), "
                     f"retrying in {delay}s...")
            time.sleep(delay)

    if isinstance(last_error, HTTPError):
        return last_error.code, last_error_body
    raise last_error

def _http_get(url: str, headers: dict = None, timeout: int = 30,
              max_retries: int = 3) -> tuple:

    return _http_request("GET", url, headers=headers,
                         timeout=timeout, max_retries=max_retries)

def _http_post(url: str, data=None, json_data=None, headers: dict = None,
               timeout: int = 30, max_retries: int = 3) -> tuple:

    return _http_request("POST", url, data=data, json_data=json_data,
                         headers=headers, timeout=timeout,
                         max_retries=max_retries)

def _is_jar_intact(path: Path) -> bool:
    try:
        if path.stat().st_size < 22:
            return False
        with open(path, "rb") as f:
            magic = f.read(4)
        return magic == b"PK\x03\x04"
    except OSError:
        return False

def _maven_rel_path(name: str) -> str:
    parts = name.split(":")
    g, a, v = parts[0], parts[1], parts[2]
    classifier = parts[3] if len(parts) > 3 else ""
    jar_name = f"{a}-{v}"
    if classifier:
        jar_name += f"-{classifier}"
    jar_name += ".jar"
    return f"{g.replace('.', '/')}/{a}/{v}/{jar_name}"

def _download_file(url: str, dest_path, label: str = "", sha1: str = None,
                   max_retries: int = 3, show_progress: bool = True,
                   extra_headers: dict = None):
    dest = Path(dest_path)
    dest.parent.mkdir(parents=True, exist_ok=True)
    label = label or dest.name

    if dest.exists():
        if sha1:
            if _sha1_file(dest) == sha1:
                return

        elif _is_jar_intact(dest):
            return
        elif dest.stat().st_size > 0:

            log.warn(f"{label} appears corrupted, re-downloading...")
        dest.unlink(missing_ok=True)

    headers = {"User-Agent": f"{LAUNCHER_NAME}/{LAUNCHER_VER}"}
    if extra_headers:
        headers.update(extra_headers)
    for attempt in range(max_retries):
        try:
            req = Request(url, headers=headers)
            with urlopen(req, timeout=120) as resp:
                try:
                    total = int(resp.headers.get("Content-Length", 0) or 0)
                except (ValueError, TypeError):
                    total = 0
                dl = 0
                with open(dest, "wb") as f:
                    while True:
                        chunk = resp.read(65536)
                        if not chunk:
                            break
                        f.write(chunk)
                        dl += len(chunk)
                        if show_progress and total:
                            pct = min(dl * 100 // total, 100)
                            filled = pct * 25 // 100
                            bar = "\u2588" * filled + "\u2591" * (25 - filled)
                            print(f"\r  {label:40s} {bar} {pct:3d}%",
                                  end="", flush=True)
                if show_progress:
                    if total:
                        bar_done = "\u2588" * 25
                        mb = dl / 1024 / 1024
                        print(f"\r  {label:40s} {bar_done} {mb:.1f} MB")
                    else:
                        print(f"\r  {label:40s} done ({dl/1024:.0f} KB)")

            if sha1:
                if _sha1_file(dest) != sha1:
                    dest.unlink()
                    log.warn(f"SHA-1 mismatch for {label}, retrying...")
                    continue
            return
        except Exception as e:
            if dest.exists():
                dest.unlink()
            if attempt < max_retries - 1:
                delay = 2 ** (attempt + 1)
                log.warn(f"{label} failed ({type(e).__name__}), "
                         f"retry {attempt+2}/{max_retries} in {delay}s...")
                time.sleep(delay)
            else:
                raise RuntimeError(
                    f"Download failed after {max_retries} attempts: {label} — {e}"
                )

class ModrinthAPI:
    @staticmethod
    def _get_json(url: str, params: dict = None):
        full = f"{url}?{urlencode(params)}" if params else url
        status, body = _http_get(full, headers={"User-Agent": f"{LAUNCHER_NAME}/{LAUNCHER_VER}"})
        if status != 200:
            log.die(f"Modrinth API returned {status} for {url}", 
                    hint=body.decode(errors="replace")[:300])
        return json.loads(body)

    @staticmethod
    def list_game_versions():
        return ModrinthAPI._get_json(f"{MODRINTH_API}/tag/game_version")

    @staticmethod
    def list_loaders():
        data = ModrinthAPI._get_json(f"{MODRINTH_API}/tag/loader")
        return [item["name"] for item in data]

    @staticmethod
    def search_projects(query, index="relevance", limit=20, offset=0,
                        facets=None, filters=None):
        params = {
            "query": query,
            "index": index,
            "limit": limit,
            "offset": offset,
        }
        if facets:

            params["facets"] = json.dumps(facets)
        return ModrinthAPI._get_json(f"{MODRINTH_API}/search", params)

    @staticmethod
    def get_project(slug_or_id):
        return ModrinthAPI._get_json(f"{MODRINTH_API}/project/{slug_or_id}")

    @staticmethod
    def get_project_versions(slug_or_id, loaders=None, game_versions=None, featured=None):
        params = {}
        if loaders:
            params["loaders"] = json.dumps(loaders)
        if game_versions:
            params["game_versions"] = json.dumps(game_versions)
        if featured is not None:
            params["featured"] = str(featured).lower()
        return ModrinthAPI._get_json(f"{MODRINTH_API}/project/{slug_or_id}/version", params)

    @staticmethod
    def get_version(version_id):
        return ModrinthAPI._get_json(f"{MODRINTH_API}/version/{version_id}")

    @staticmethod
    def get_versions(version_ids):
        params = {"ids": json.dumps(version_ids)}
        return ModrinthAPI._get_json(f"{MODRINTH_API}/versions", params)

    @staticmethod
    def download_version_files(version_data, dest_dir, label=""):
        dest = Path(dest_dir)
        dest.mkdir(parents=True, exist_ok=True)
        paths = []
        files = version_data.get("files", [])

        primary_files = [f for f in files if f.get("primary")]
        if not primary_files:

            primary_files = [f for f in files
                             if not f["filename"].endswith("-sources.jar")
                             and not f["filename"].endswith("-javadoc.jar")]

        downloaded_names = set()
        for f in primary_files:
            file_path = dest / f["filename"]
            if not file_path.exists():
                _download_file(f["url"], file_path, label or f["filename"])
            else:
                print(f"  {f['filename']} — cached")
            paths.append(file_path)
            downloaded_names.add(f["filename"])

        if downloaded_names:
            for suffix in ("-sources.jar", "-javadoc.jar"):
                for old in dest.glob(f"*{suffix}"):
                    base = old.name[:-len(suffix)] + ".jar"
                    if base in downloaded_names and old.exists():
                        old.unlink()
                        log.info(f"Removed stale {suffix[1:-4]} jar: {old.name}")

        return paths


class ModrinthSource:
    name = "modrinth"

    def search(self, query, limit=10, offset=0, loader=None, game_version=None):
        facets = [["project_type:mod"]]
        if loader:
            facets.append([f"categories:{loader.lower()}"])
        if game_version:
            facets.append([f"versions:{game_version}"])
        result = ModrinthAPI.search_projects(query, index="relevance",
                                              limit=limit, offset=offset,
                                              facets=facets)
        hits = result.get("hits", [])
        for h in hits:
            h["source"] = "modrinth"
        return hits

    def get_project(self, slug_or_id):
        data = ModrinthAPI.get_project(slug_or_id)
        data["source"] = "modrinth"
        return data

    def get_versions(self, project_id, loader=None, game_version=None, limit=50):
        kwargs = {}
        if loader:
            kwargs["loaders"] = [loader.lower()]
        if game_version:
            kwargs["game_versions"] = [game_version]
        versions = ModrinthAPI.get_project_versions(project_id, **kwargs)
        for v in versions:
            v["source"] = "modrinth"
        return versions

    def download(self, version_data, dest_dir, label=""):
        return ModrinthAPI.download_version_files(version_data, dest_dir, label)


class FabricManager:
    FABRIC_META = "https://meta.fabricmc.net/v2"

    def __init__(self, game_dir: Path):
        self.game_dir = game_dir
        self.lib_dir = game_dir / "libraries"

    def get_available_versions(self, mc_version=None):
        if mc_version:
            url = f"{self.FABRIC_META}/versions/loader/{mc_version}"
        else:
            url = f"{self.FABRIC_META}/versions/loader"
        status, body = _http_get(url)
        if status != 200:
            log.die(f"Fabric Meta API returned {status}")
        return json.loads(body)

    def _fetch_profile(self, mc_version, loader_version):
        url = f"{self.FABRIC_META}/versions/loader/{mc_version}/{loader_version}/profile/json"
        status, body = _http_get(url)
        if status != 200:
            log.die(f"Fabric profile fetch failed ({status}) for {mc_version}/{loader_version}")
        return json.loads(body)

    @staticmethod
    def _maven_path(name):
        parts = name.split(":")
        g, a, v = parts[0], parts[1], parts[2]
        return f"{g.replace('.', '/')}/{a}/{v}/{a}-{v}.jar"

    def install(self, mc_version, loader_version_id=None):
        versions = self.get_available_versions(mc_version)
        if not versions:
            log.die(f"No Fabric loader found for Minecraft {mc_version}")

        if loader_version_id:
            target = None
            for v in versions:
                if v.get("loader", {}).get("version") == loader_version_id:
                    target = v
                    break
            if not target:
                log.die(f"Fabric loader version '{loader_version_id}' not found for MC {mc_version}")
        else:
            target = versions[0]

        loader_ver = target["loader"]["version"]
        log.info(f"Installing Fabric Loader {loader_ver} for Minecraft {mc_version}...")

        profile = self._fetch_profile(mc_version, loader_ver)
        libraries = profile.get("libraries", [])
        log.info(f"Profile: {len(libraries)} libraries to download")

        all_jars = []
        for lib in libraries:
            name = lib["name"]
            url_base = lib.get("url", "https://maven.fabricmc.net/")
            rel_path = self._maven_path(name)
            jar_path = self.lib_dir / rel_path

            if not jar_path.exists():
                full_url = url_base.rstrip("/") + "/" + rel_path
                label = name.split(":")[1]
                _download_file(full_url, jar_path, label)
            all_jars.append(jar_path)

        log.success(f"Fabric {loader_ver} installed ({len(all_jars)} jars)")

        profile_path = self.lib_dir / "fabric" / f"fabric-profile-{mc_version}.json"
        profile_path.parent.mkdir(parents=True, exist_ok=True)
        profile_path.write_text(json.dumps(profile, indent=2), encoding="utf-8")

        return all_jars, profile


class ForgeManager:
    PROMOTIONS_URL = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json"
    MAVEN_META_URL = "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml"

    def __init__(self, game_dir: Path):
        self.game_dir = game_dir
        self.lib_dir = game_dir / "libraries"

    def get_available_versions(self, mc_version):
        status, body = _http_get(self.MAVEN_META_URL)
        if status != 200:
            log.die(f"Forge maven metadata fetch failed ({status})")
        import xml.etree.ElementTree as ET
        root = ET.fromstring(body)
        versions = []
        prefix = f"{mc_version}-"
        for v in root.findall(".//version"):
            text = v.text
            if text and text.startswith(prefix):
                versions.append(text[len(prefix):])
        versions.sort(key=lambda x: [int(n) for n in re.findall(r'(\d+)', x) or [0]])
        return versions

    def get_promotions(self):
        status, body = _http_get(self.PROMOTIONS_URL)
        if status != 200:
            return {}
        try:
            return json.loads(body).get("promos", {})
        except Exception:
            return {}

    def get_recommended_version(self, mc_version):
        promos = self.get_promotions()
        return promos.get(f"{mc_version}-recommended") or promos.get(f"{mc_version}-latest")

    def installer_url(self, mc_version, loader_version):
        return (f"{FORGE_MAVEN}/{mc_version}-{loader_version}/"
                f"forge-{mc_version}-{loader_version}-installer.jar")

    def _ensure_base_game(self, mc_version):
        vm = VersionManager(self.game_dir)
        version_id, version_data = vm.get_version_info(mc_version)
        vm.download_client_jar(version_id, version_data)
        profile_path = self.game_dir / "launcher_profiles.json"
        if not profile_path.exists():
            profile_path.write_text("{}", encoding="utf-8")

    def install(self, mc_version, loader_version_id=None):
        versions = self.get_available_versions(mc_version)
        if not versions:
            log.die(f"No Forge loader found for Minecraft {mc_version}")

        if loader_version_id:
            if loader_version_id not in versions:
                log.die(f"Forge loader version '{loader_version_id}' not found for MC {mc_version}")
            loader_ver = loader_version_id
        else:
            rec = self.get_recommended_version(mc_version)
            loader_ver = rec if rec and rec in versions else versions[-1]

        log.info(f"Installing Forge {loader_ver} for Minecraft {mc_version}...")
        self._ensure_base_game(mc_version)

        installer_url = self.installer_url(mc_version, loader_ver)
        installer_path = self.lib_dir / "forge" / f"forge-{mc_version}-{loader_ver}-installer.jar"
        installer_path.parent.mkdir(parents=True, exist_ok=True)
        _download_file(installer_url, installer_path, f"Forge {loader_ver} installer")

        java = check_java()
        cmd = [java, "-jar", str(installer_path), "--installClient", str(self.game_dir)]
        log.info("Running Forge installer (this may take a while)...")
        try:
            subprocess.run(cmd, check=True)
        except subprocess.CalledProcessError as e:
            log.die(f"Forge installer failed (exit {e.returncode})")

        installed_id = f"{mc_version}-forge-{loader_ver}"
        version_json_path = self.game_dir / "versions" / installed_id / f"{installed_id}.json"
        if not version_json_path.exists():
            candidates = sorted((self.game_dir / "versions").glob(f"{mc_version}-forge-*"))
            if candidates:
                installed_id = candidates[-1].name
                version_json_path = candidates[-1] / f"{installed_id}.json"
        if not version_json_path.exists():
            log.die("Forge installer finished but version.json was not found.")

        profile = self._build_profile(mc_version, installed_id, version_json_path)
        profile_path = self.lib_dir / "forge" / f"forge-profile-{mc_version}.json"
        profile_path.parent.mkdir(parents=True, exist_ok=True)
        profile_path.write_text(json.dumps(profile, indent=2), encoding="utf-8")

        log.success(f"Forge {loader_ver} installed for Minecraft {mc_version}")
        return installed_id, profile

    def _build_profile(self, mc_version, version_id, version_json_path):
        version_data = json.loads(version_json_path.read_text(encoding="utf-8"))
        merged = self._resolve_inherits(version_data, self.game_dir / "versions")
        return {
            "loader": "forge",
            "mc_version": mc_version,
            "version_id": version_id,
            "mainClass": merged.get("mainClass"),
            "libraries": merged.get("libraries", []),
            "arguments": merged.get("arguments", {}),
        }

    def _resolve_inherits(self, version_data, versions_dir):
        parent_id = version_data.get("inheritsFrom")
        if not parent_id:
            return version_data
        parent_path = versions_dir / parent_id / f"{parent_id}.json"
        if not parent_path.exists():
            log.warn(f"Parent version {parent_id} not found; Forge may not launch.")
            return version_data
        parent = json.loads(parent_path.read_text(encoding="utf-8"))
        merged_parent = self._resolve_inherits(parent, versions_dir)

        merged = dict(merged_parent)
        merged.update({k: v for k, v in version_data.items()
                       if k != "libraries"})
        merged["libraries"] = merged_parent.get("libraries", []) + version_data.get("libraries", [])
        return merged


class NeoForgeManager:
    MAVEN_META_URL = "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml"

    def __init__(self, game_dir: Path):
        self.game_dir = game_dir
        self.lib_dir = game_dir / "libraries"

    @staticmethod
    def _neoforge_prefix_for_mc(mc_version):
        parts = mc_version.split(".")
        if len(parts) >= 3 and parts[0] == "1" and parts[1] == "20":
            patch = parts[2]
            if patch == "1":
                # 1.20.1 NeoForge builds were published under net.neoforged:forge,
                # not net.neoforged:neoforge; use Forge loader for this MC version.
                return None
            return f"20.{patch}"
        if len(parts) >= 2 and parts[0] == "1" and int(parts[1]) >= 21:
            minor = parts[1]
            patch = parts[2] if len(parts) >= 3 else "0"
            return f"{minor}.{patch}"
        return None

    def get_available_versions(self, mc_version):
        status, body = _http_get(self.MAVEN_META_URL)
        if status != 200:
            log.die(f"NeoForge maven metadata fetch failed ({status})")
        import xml.etree.ElementTree as ET
        root = ET.fromstring(body)
        prefix = self._neoforge_prefix_for_mc(mc_version)
        if not prefix:
            log.die(f"Cannot determine NeoForge versions for Minecraft {mc_version}. "
                    f"Specify --loader-version.")
        versions = []
        for v in root.findall(".//version"):
            text = v.text
            if text and text.startswith(prefix):
                rest = text[len(prefix):]
                # Prefixes ending in a separator (e.g. "47.") are followed directly by digits.
                # Prefixes like "21.4" must be followed by ".<digit>" to avoid matching 21.40.
                if prefix.endswith((".", "-")):
                    if rest and rest[0].isdigit():
                        versions.append(text)
                elif rest.startswith(".") and len(rest) > 1 and rest[1].isdigit():
                    versions.append(text)
        versions.sort(key=lambda x: [int(n) for n in re.findall(r'(\d+)', x) or [0]])
        return versions

    def installer_url(self, loader_version):
        return f"{NEOFORGE_MAVEN}/{loader_version}/neoforge-{loader_version}-installer.jar"

    def _ensure_base_game(self, mc_version):
        vm = VersionManager(self.game_dir)
        version_id, version_data = vm.get_version_info(mc_version)
        vm.download_client_jar(version_id, version_data)
        profile_path = self.game_dir / "launcher_profiles.json"
        if not profile_path.exists():
            profile_path.write_text("{}", encoding="utf-8")

    def install(self, mc_version, loader_version_id=None):
        versions = self.get_available_versions(mc_version)
        if not versions:
            log.die(f"No NeoForge loader found for Minecraft {mc_version}")

        if loader_version_id:
            if loader_version_id not in versions:
                log.die(f"NeoForge loader version '{loader_version_id}' not found.")
            loader_ver = loader_version_id
        else:
            loader_ver = versions[-1]

        log.info(f"Installing NeoForge {loader_ver} for Minecraft {mc_version}...")
        self._ensure_base_game(mc_version)

        installer_url = self.installer_url(loader_ver)
        installer_path = self.lib_dir / "neoforge" / f"neoforge-{loader_ver}-installer.jar"
        installer_path.parent.mkdir(parents=True, exist_ok=True)
        _download_file(installer_url, installer_path, f"NeoForge {loader_ver} installer")

        java = check_java()
        cmd = [java, "-jar", str(installer_path), "--installClient", str(self.game_dir)]
        log.info("Running NeoForge installer (this may take a while)...")
        try:
            subprocess.run(cmd, check=True)
        except subprocess.CalledProcessError as e:
            log.die(f"NeoForge installer failed (exit {e.returncode})")

        installed_id = f"neoforge-{loader_ver}"
        version_json_path = self.game_dir / "versions" / installed_id / f"{installed_id}.json"
        if not version_json_path.exists():
            candidates = sorted((self.game_dir / "versions").glob("neoforge-*"))
            if candidates:
                installed_id = candidates[-1].name
                version_json_path = candidates[-1] / f"{installed_id}.json"
        if not version_json_path.exists():
            log.die("NeoForge installer finished but version.json was not found.")

        profile = self._build_profile(mc_version, installed_id, version_json_path)
        profile_path = self.lib_dir / "neoforge" / f"neoforge-profile-{mc_version}.json"
        profile_path.parent.mkdir(parents=True, exist_ok=True)
        profile_path.write_text(json.dumps(profile, indent=2), encoding="utf-8")

        log.success(f"NeoForge {loader_ver} installed for Minecraft {mc_version}")
        return installed_id, profile

    def _build_profile(self, mc_version, version_id, version_json_path):
        version_data = json.loads(version_json_path.read_text(encoding="utf-8"))
        merged = self._resolve_inherits(version_data, self.game_dir / "versions")
        return {
            "loader": "neoforge",
            "mc_version": mc_version,
            "version_id": version_id,
            "mainClass": merged.get("mainClass"),
            "libraries": merged.get("libraries", []),
            "arguments": merged.get("arguments", {}),
        }

    def _resolve_inherits(self, version_data, versions_dir):
        parent_id = version_data.get("inheritsFrom")
        if not parent_id:
            return version_data
        parent_path = versions_dir / parent_id / f"{parent_id}.json"
        if not parent_path.exists():
            log.warn(f"Parent version {parent_id} not found; NeoForge may not launch.")
            return version_data
        parent = json.loads(parent_path.read_text(encoding="utf-8"))
        merged_parent = self._resolve_inherits(parent, versions_dir)

        merged = dict(merged_parent)
        merged.update({k: v for k, v in version_data.items()
                       if k != "libraries"})
        merged["libraries"] = merged_parent.get("libraries", []) + version_data.get("libraries", [])
        return merged


class ModManager:
    def __init__(self, game_dir: Path):
        self.game_dir = game_dir
        self.modrinth = ModrinthSource()

    def _mods_dir(self, mc_version):
        d = self.game_dir / "versions" / mc_version / "mods"
        d.mkdir(parents=True, exist_ok=True)
        return d

    @staticmethod
    def list_installed_versions(game_dir: Path):
        versions_dir = game_dir / "versions"
        if not versions_dir.exists():
            return []
        vers = []
        for d in versions_dir.iterdir():
            if d.is_dir() and (d / f"{d.name}.json").exists():
                vers.append(d.name)

        def _key(v):
            parts = re.findall(r'(\d+)', v)
            return tuple(int(p) for p in parts) if parts else (0,)
        vers.sort(key=_key)
        return vers

    def search(self, query, limit=10, game_version=None, loader=None):
        return self.modrinth.search(query, limit=limit, loader=loader,
                                    game_version=game_version)

    LOADERS_SHOWN = ("fabric", "forge", "neoforge")
    _RELEASE_MC_RE = re.compile(r'^\d+\.\d+(\.\d+)?$')

    @staticmethod
    def _mc_key(v):
        parts = re.findall(r'\d+', v)
        return tuple(int(p) for p in parts) if parts else (0,)

    @classmethod
    def summarize_loader_support(cls, versions):
        """Map loader -> (highest release MC version, mod version supporting it)."""
        support = {}
        for v in versions:
            gvs = [g for g in v.get("game_versions", [])
                   if cls._RELEASE_MC_RE.match(g)] or v.get("game_versions", [])
            if not gvs:
                continue
            top = max(gvs, key=cls._mc_key)
            for l in v.get("loaders", []):
                l = l.lower()
                cur = support.get(l)
                if cur is None or cls._mc_key(top) > cls._mc_key(cur[0]):
                    support[l] = (top, v.get("version_number", v.get("id", "?")))
        return support

    def loader_support(self, project_id):
        try:
            return self.summarize_loader_support(self.modrinth.get_versions(project_id))
        except SystemExit:
            raise
        except Exception:
            return {}

    @classmethod
    def format_loader_support(cls, support):
        parts = []
        for l in cls.LOADERS_SHOWN:
            if l in support:
                mc, modver = support[l]
                parts.append(f"{l} <= MC {mc} ({modver})")
            else:
                parts.append(f"{l} \u2014")
        return "  |  ".join(parts)

    def _detect_loaders(self, mc_version):
        found = []
        for loader in ("fabric", "neoforge", "forge"):
            p = self.game_dir / "libraries" / loader / f"{loader}-profile-{mc_version}.json"
            if p.exists():
                found.append(loader)
        return found

    def _pick_loader(self, mc_version, preferred=None):
        if preferred:
            return preferred
        detected = self._detect_loaders(mc_version)
        if not detected:
            return None
        if len(detected) == 1:
            return detected[0]
        order = ["fabric", "neoforge", "forge"]
        for loader in order:
            if loader in detected:
                log.info(f"Multiple loaders detected; picked {loader}. Use --loader to override.")
                return loader
        return detected[0]

    def install(self, slug, mc_version, loader=None, version_id=None):
        log.info(f"Resolving mod: {slug}...")
        loader = self._pick_loader(mc_version, loader)
        if loader:
            log.info(f"Using loader: {loader}")

        project, versions = self._resolve_mod(slug, mc_version, loader)
        proj_title = project.get("title", slug)

        if version_id:
            target = None
            for v in versions:
                if str(v["id"]) == str(version_id):
                    target = v
                    break
            if not target:
                log.die(f"Version '{version_id}' not found for {proj_title}")
        else:
            def _loader_match(v):
                loaders = [l.lower() for l in v.get("loaders", [])]
                return 0 if loader and loader.lower() in loaders else 1
            versions.sort(key=lambda v: v.get("date_published", ""), reverse=True)
            versions.sort(key=_loader_match)
            target = versions[0]

        ver_num = target.get("version_number", target["id"])
        mc_str = ", ".join(target.get("game_versions", ["?"]))
        loaders_str = ", ".join(target.get("loaders", ["?"]))
        if loader and loader.lower() not in [l.lower() for l in target.get("loaders", [])]:
            log.warn(f"Note: selected version uses loader '{loaders_str}', not '{loader}'")
        log.info(f"Installing {proj_title}...")
        print(f"    Mod version:   {ver_num}")
        print(f"    Game versions: {mc_str}")
        print(f"    Loaders:       {loaders_str}")

        self._check_dependencies(target, mc_version, loader)

        dest_dir = self._mods_dir(mc_version)
        paths = self.modrinth.download(target, dest_dir, f"{proj_title} {ver_num}")
        total_size = sum(p.stat().st_size for p in paths if p.exists())
        log.success(f"Installed {len(paths)} file(s) ({total_size / 1024:.1f} KB) -> {dest_dir}")
        return paths, target, project

    def _check_dependencies(self, version_data, mc_version, loader):
        """检测模组依赖并醒目打印，提醒用户手动安装缺失的 required 依赖。"""
        deps = [d for d in version_data.get("dependencies", []) if d.get("project_id")]
        if not deps:
            return

        dest_dir = self._mods_dir(mc_version)
        existing_jars = [f.name.lower() for f in dest_dir.glob("*.jar")
                         if not f.name.endswith(("-sources.jar", "-javadoc.jar", ".disabled"))]

        required_missing = []
        optional_list = []
        for dep in deps:
            dep_pid = dep["project_id"]
            dep_type = dep.get("dependency_type", "required")
            try:
                dep_proj = self.modrinth.get_project(dep_pid)
            except Exception as e:
                log.warn(f"Could not resolve dependency {dep_pid}: {e}")
                continue
            dep_title = dep_proj.get("title", dep_pid)
            dep_slug = dep_proj.get("slug", dep_pid)

            dep_versions = self.modrinth.get_versions(dep_pid, loader=loader,
                                                      game_version=mc_version)
            if dep_versions:
                def _dk(v):
                    loaders = [l.lower() for l in v.get("loaders", [])]
                    return 0 if loader and loader.lower() in loaders else 1
                dep_versions.sort(key=lambda v: v.get("date_published", ""), reverse=True)
                dep_versions.sort(key=_dk)
                dep_ver_num = dep_versions[0].get("version_number", dep_versions[0]["id"])
                dep_mc = ", ".join(dep_versions[0].get("game_versions", ["?"]))
            else:
                dep_ver_num = "(no matching version)"
                dep_mc = "?"

            dep_key_lower = dep_slug.lower()
            dep_title_lower = dep_title.lower().replace(" ", "_")
            present = any(dep_key_lower in n or dep_title_lower in n
                          for n in existing_jars)

            if dep_type == "required":
                if present:
                    log.info(f"[dep: required] {dep_title} — already installed")
                else:
                    required_missing.append((dep_title, dep_slug, dep_ver_num, dep_mc))
            else:
                if present:
                    log.info(f"[dep: optional] {dep_title} — already installed")
                optional_list.append((dep_title, dep_slug, dep_ver_num, dep_mc, present))

        if required_missing or optional_list:
            print()
            log.header("Dependencies")
            if required_missing:
                print(f"  {_Log.BOLD}{_Log.RED}[ MUST INSTALL ] Required dependencies:{_Log.RESET}")
                for title, slug, ver, mc in required_missing:
                    print(f"    {_Log.RED}- {title}{_Log.RESET}")
                    print(f"      slug:    {slug}")
                    print(f"      version: {ver}  (MC: {mc})")
                    print(f"      install: python mc_launcher.py install-mod {slug} -v {mc_version}" +
                          (f" --loader {loader}" if loader else ""))
            if optional_list:
                print(f"  {_Log.BOLD}{_Log.YELLOW}[ RECOMMENDED ] Optional dependencies:{_Log.RESET}")
                for title, slug, ver, mc, present in optional_list:
                    mark = f"{_Log.GREEN}[installed]{_Log.RESET} " if present else ""
                    print(f"    {mark}{title}")
                    print(f"      slug:    {slug}")
                    print(f"      version: {ver}  (MC: {mc})")
                    print(f"      install: python mc_launcher.py install-mod {slug} -v {mc_version}" +
                          (f" --loader {loader}" if loader else ""))
            print()
            if required_missing:
                log.warn(f"{len(required_missing)} required dependency(ies) missing. "
                         f"Install them first, or the mod may not load.")
            print()

    def _resolve_mod(self, slug, mc_version, loader):
        project = self.modrinth.get_project(slug)
        versions = self.modrinth.get_versions(project["id"], loader=loader,
                                              game_version=mc_version)
        if versions:
            return project, versions

        proj_title = project.get("title", slug)
        extra = f" for MC {mc_version}"
        extra += f" ({loader})" if loader else ""
        log.error(f"No versions found for {proj_title}{extra}")
        support = self.loader_support(project["id"])
        if support:
            print(f"\n  Available support for '{proj_title}':")
            print(f"    {self.format_loader_support(support)}")
            others = {l: s for l, s in support.items() if l not in self.LOADERS_SHOWN}
            for l, (mc, modver) in sorted(others.items()):
                print(f"    {l} <= MC {mc} ({modver})")
            print()
        if not loader:
            print("  Hint: install a loader first, or specify --loader manually")
        sys.exit(1)

    def list_mods(self, mc_version):
        mods_dir = self._mods_dir(mc_version)
        results = []
        for f in sorted(mods_dir.iterdir()):
            name = f.name
            if name.endswith(".disabled"):
                results.append((name[:-9], False, f.stat().st_size))
            elif name.endswith(".jar"):
                results.append((name, True, f.stat().st_size))
        return results

    def disable_mod(self, slug, mc_version):
        mods_dir = self._mods_dir(mc_version)
        target = mods_dir / f"{slug}.jar"
        if target.exists():
            target.rename(target.with_name(target.name + ".disabled"))
            return True

        for f in mods_dir.glob("*.jar"):
            if slug.lower() in f.name.lower():
                f.rename(f.with_name(f.name + ".disabled"))
                return True
        return False

    def enable_mod(self, slug, mc_version):
        mods_dir = self._mods_dir(mc_version)
        target = mods_dir / f"{slug}.jar.disabled"
        if target.exists():
            target.rename(target.with_name(target.name[:-9]))
            return True
        for f in mods_dir.glob("*.jar.disabled"):
            if slug.lower() in f.name.lower():
                f.rename(f.with_name(f.name[:-9]))
                return True
        return False

    def uninstall_mod(self, slug, mc_version):
        mods_dir = self._mods_dir(mc_version)
        deleted = []
        for pattern in [f"{slug}.jar", f"{slug}.jar.disabled"]:
            target = mods_dir / pattern
            if target.exists():
                target.unlink()
                deleted.append(target.name)

        if not deleted:
            for f in list(mods_dir.glob("*.jar")) + list(mods_dir.glob("*.jar.disabled")):
                if slug.lower() in f.name.lower():
                    f.unlink()
                    deleted.append(f.name)
        return deleted

def find_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

def _java_major(java_path):
    try:
        out = subprocess.check_output([str(java_path), "-version"],
                                      stderr=subprocess.STDOUT, text=True, timeout=15)
        m = re.search(r'version "(\d+)(?:\.(\d+))?', out)
        if not m:
            return None
        major = int(m.group(1))
        if major == 1 and m.group(2):
            major = int(m.group(2))
        return major
    except Exception:
        return None

def _find_java_candidates():
    candidates = []
    java_bin = "java.exe" if platform.system() == "Windows" else "java"

    jh = os.environ.get("JAVA_HOME", "")
    if jh:
        je = Path(jh) / "bin" / java_bin
        if je.exists():
            candidates.append(str(je))

    w = shutil.which("java")
    if w:
        candidates.append(w)

    scanned = []
    if platform.system() == "Windows":
        import winreg
        reg_roots = [
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Eclipse Adoptium\JDK"),
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Eclipse Foundation\JDK"),
            (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\JavaSoft\JDK"),
            (winreg.HKEY_CURRENT_USER,  r"SOFTWARE\Eclipse Adoptium\JDK"),
            (winreg.HKEY_CURRENT_USER,  r"SOFTWARE\JavaSoft\JDK"),
        ]
        for root, key_path in reg_roots:
            try:
                with winreg.OpenKey(root, key_path) as jdk_key:
                    i = 0
                    while True:
                        try:
                            sub_key = winreg.EnumKey(jdk_key, i)
                            with winreg.OpenKey(jdk_key, sub_key) as sk:
                                jh_reg, _ = winreg.QueryValueEx(sk, "Path")
                                je = Path(jh_reg) / "bin" / "java.exe"
                                if je.exists():
                                    scanned.append(str(je))
                            i += 1
                        except OSError:
                            break
            except OSError:
                continue

        user_home = Path(os.environ.get("USERPROFILE", Path.home()))
        for d in sorted(user_home.glob("jdk-*"), reverse=True):
            je = d / "bin" / "java.exe"
            if je.exists():
                scanned.append(str(je))

        for base in [r"C:\Program Files\Java", r"C:\Program Files\Eclipse Adoptium",
                     r"C:\Program Files\Microsoft", r"C:\Program Files\Eclipse Foundation",
                     r"C:\Program Files (x86)\Java"]:
            bp = Path(base)
            if bp.exists():
                for d in bp.glob("*"):
                    if d.is_dir():
                        je = d / "bin" / "java.exe"
                        if je.exists() and str(je) not in scanned:
                            scanned.append(str(je))
                        for sd in d.glob("*"):
                            if sd.is_dir():
                                je2 = sd / "bin" / "java.exe"
                                if je2.exists() and str(je2) not in scanned:
                                    scanned.append(str(je2))
    else:
        for base in ["/usr/lib/jvm", "/usr/local/opt", Path.home() / ".sdkman/candidates/java",
                     Path.home() / ".jdks"]:
            bp = Path(base)
            if bp.exists():
                for d in sorted(bp.glob("*"), reverse=True):
                    je = d / "bin" / "java"
                    if je.exists():
                        scanned.append(str(je))

        hb = Path("/usr/local/opt/openjdk/bin/java")
        if hb.exists():
            scanned.append(str(hb))

    def _key(p):
        nums = re.findall(r'(\d+)', p)
        return tuple(int(n) for n in nums) if nums else (0,)
    scanned.sort(key=_key, reverse=True)

    seen = set()
    result = []
    for c in candidates + scanned:
        try:
            key = str(Path(c).resolve())
        except OSError:
            key = c
        if key not in seen:
            seen.add(key)
            result.append(c)
    return result

def check_java(required_major=None):
    candidates = _find_java_candidates()

    if required_major:
        for c in candidates:
            if _java_major(c) == required_major:
                return c
        return None

    if not candidates:
        log.die("Java not found. Install Java 17+ from https://adoptium.net/",
                hint='If you already installed Java, set JAVA_HOME:\n         PowerShell: $env:JAVA_HOME = "C:\\path\\to\\jdk"')

    java = candidates[0]
    ver = _java_major(java)
    if ver and ver < 17:
        log.warn(f"Java {ver} detected. Minecraft 1.18+ needs Java 17+.")
        print(f"  Found at: {java}")
    return java

MOJANG_JAVA_MANIFEST = ("https://launchermeta.mojang.com/v1/products/java-runtime/"
                        "2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json")

def _mojang_java_platform():
    osn = os_name()
    arch = os_arch()
    if osn == "windows":
        return "windows-arm64" if arch == "arm64" else "windows-x64"
    if osn == "osx":
        return "mac-os-arm64" if arch == "arm64" else "mac-os"
    if osn == "linux" and arch == "x86_64":
        return "linux"
    return None

def download_mojang_java(game_dir: Path, component: str, max_workers=4):
    """Download Mojang's bundled Java runtime (same as the official launcher).
    Returns path to the java executable, or None on failure."""
    dest_root = game_dir / "java" / component
    exe_name = "java.exe" if platform.system() == "Windows" else "java"

    existing = sorted(dest_root.glob(f"**/bin/{exe_name}"))
    if existing:
        return str(existing[0])

    plat = _mojang_java_platform()
    if not plat:
        log.warn(f"No Mojang Java runtime available for {os_name()}/{os_arch()}.")
        return None

    try:
        status, body = _http_get(MOJANG_JAVA_MANIFEST)
        if status != 200:
            log.warn(f"Mojang Java runtime index fetch failed ({status})")
            return None
        entries = json.loads(body).get(plat, {}).get(component, [])
        if not entries:
            log.warn(f"Mojang has no Java runtime '{component}' for platform '{plat}'.")
            return None
        ver_name = entries[0].get("version", {}).get("name", "?")
        status, body = _http_get(entries[0]["manifest"]["url"])
        if status != 200:
            log.warn(f"Mojang Java runtime manifest fetch failed ({status})")
            return None
        files = json.loads(body).get("files", {})
    except Exception as e:
        log.warn(f"Mojang Java runtime metadata error: {e}")
        return None

    downloads = []
    links = []
    executables = []
    for rel, info in sorted(files.items()):
        target = dest_root / rel
        ftype = info.get("type")
        if ftype == "directory":
            target.mkdir(parents=True, exist_ok=True)
        elif ftype == "file":
            raw = info.get("downloads", {}).get("raw")
            if not raw:
                continue
            if not (target.exists() and raw.get("sha1")
                    and _sha1_file(target) == raw["sha1"]):
                downloads.append((raw["url"], target, raw.get("sha1")))
            if info.get("executable"):
                executables.append(target)
        elif ftype == "link" and info.get("target"):
            links.append((target, info["target"]))

    log.info(f"Downloading Java runtime {ver_name} ({component}): "
             f"{len(downloads)} files [{max_workers} threads]...")

    fail = [0]
    done = [0]
    lock = threading.Lock()

    def _fetch(url, dest, sha1):
        try:
            _download_file(url, dest, "", sha1=sha1, show_progress=False)
            return True
        except Exception:
            return False

    if downloads:
        with ThreadPoolExecutor(max_workers=max_workers) as pool:
            futures = {pool.submit(_fetch, u, d, s): d for u, d, s in downloads}
            for f in as_completed(futures):
                with lock:
                    if f.result():
                        done[0] += 1
                    else:
                        fail[0] += 1
                    n = done[0] + fail[0]
                    pct = n * 100 // len(downloads)
                    filled = pct * 25 // 100
                    bar = "\u2588" * filled + "\u2591" * (25 - filled)
                    print(f"\r  [{pct:3d}%] {bar} {n}/{len(downloads)} files",
                          end="", flush=True)
        print()

    if fail[0]:
        log.warn(f"{fail[0]} Java runtime file(s) failed to download.")
        return None

    if platform.system() != "Windows":
        for t in executables:
            try:
                t.chmod(t.stat().st_mode | 0o755)
            except OSError:
                pass
        for target, link_target in links:
            try:
                if not target.exists() and not target.is_symlink():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    os.symlink(link_target, target)
            except OSError:
                pass

    java_bins = sorted(dest_root.glob(f"**/bin/{exe_name}"))
    if not java_bins:
        log.warn("Java runtime downloaded but java executable not found.")
        return None
    log.success(f"Java runtime {ver_name} installed -> {dest_root}")
    return str(java_bins[0])

def os_name():
    p = platform.system().lower()
    if p == "windows": return "windows"
    if p == "darwin":  return "osx"
    return "linux"

def os_arch():
    m = platform.machine().lower()
    if m in ("x86_64", "amd64"): return "x86_64"
    if m in ("aarch64", "arm64"): return "arm64"
    return m

def offline_uuid(username):
    name = f"OfflinePlayer:{username}"
    md5 = bytearray(hashlib.md5(name.encode("utf-8")).digest())
    md5[6] = (md5[6] & 0x0F) | 0x30
    md5[8] = (md5[8] & 0x3F) | 0x80
    h = md5.hex()
    return f"{h[0:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"

class AccountManager:
    def __init__(self, game_dir):
        self.path = game_dir / ACCOUNTS_FILE
        self.data = {"accounts": {}, "default": None}
        if self.path.exists():
            try:
                self.data = json.loads(self.path.read_text(encoding="utf-8"))
            except Exception:
                pass

    def save(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(json.dumps(self.data, indent=2), encoding="utf-8")

    def set_msa(self, username, uid, access_token, refresh_token, expires_at):
        self.data["accounts"]["msa"] = {
            "type": "msa",
            "username": username,
            "uuid": uid,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expires_at": expires_at,
        }
        self.data["default"] = "msa"
        self.save()

    def set_offline(self, username):
        uid = offline_uuid(username)
        self.data["accounts"]["offline"] = {
            "type": "offline",
            "username": username,
            "uuid": uid,
        }
        self.data["default"] = "offline"
        self.save()

    def get_default(self):
        key = self.data.get("default")
        if key and key in self.data.get("accounts", {}):
            return self.data["accounts"][key]

        for k in ("msa", "offline"):
            if k in self.data.get("accounts", {}):
                return self.data["accounts"][k]
        return None

    def clear(self):
        self.data = {"accounts": {}, "default": None}
        self.save()

class MicrosoftAuth:
    def __init__(self):
        self.access_token  = None
        self.refresh_token = None
        self.expires_at    = 0
        self.mc_token      = None
        self.username      = None
        self.uuid          = None

    def login(self):

        params = {
            "client_id": MS_CLIENT_ID,
            "response_type": "code",
            "scope": MS_SCOPE,
            "redirect_uri": MS_REDIRECT,
            "prompt": "select_account",
            "lw": "1",
            "fl": "dob,easi2",
            "xsup": "1",
            "nopa": "2",
        }
        auth_url = f"{MS_AUTH_URL}?{urlencode(params)}"

        print(f"""
  ┌─────────────────────────────────────────────────────────────┐
  │  [1/6] Microsoft Login                                      │
  │                                                             │
  │  A browser will open. Sign in with your Microsoft account.  │
  │  After login, you'll be redirected to a BLANK PAGE.         │
  │  COPY the FULL URL from the address bar, and PASTE it below.│
  │                                                             │
  │  If the browser doesn't open, go to:                        │
  │    {auth_url}
  └─────────────────────────────────────────────────────────────┘
""")
        webbrowser.open(auth_url)

        try:
            redirect_url = input(log._c(log.CYAN, "  Paste redirect URL here: ")).strip()
        except (EOFError, KeyboardInterrupt):
            log.die("Cancelled.")

        if not redirect_url:
            log.die("No URL provided.")

        parsed = parse_qs(urlparse(redirect_url).query)
        auth_code_list = parsed.get("code", [])
        if not auth_code_list:
            log.die("Could not find 'code' in the URL.\n  Make sure you copied the FULL URL from the address bar.")
        auth_code = auth_code_list[0]

        log.step(2, 6, "Exchanging auth code for Microsoft token...")
        status, body = _http_post(MS_TOKEN_URL, data=urlencode({
            "client_id": MS_CLIENT_ID,
            "code": auth_code,
            "grant_type": "authorization_code",
            "redirect_uri": MS_REDIRECT,
            "scope": MS_SCOPE,
        }))
        if status != 200:
            log.die(f"Token exchange failed ({status})", 
                    hint=body.decode(errors="replace")[:500])
        ms_data = json.loads(body)
        self.access_token  = ms_data["access_token"]
        self.refresh_token = ms_data["refresh_token"]
        self.expires_at    = time.time() + ms_data.get("expires_in", 3600)

        self.mc_token, self.username, self.uuid, self.expires_at = \
            self.do_full_auth_chain(self.access_token)

        log.success(f"Logged in as: {self.username} ({self.uuid})")
        return True

    def device_code_login(self):

        print(f"""
  ┌─────────────────────────────────────────────────────────────┐
  │  [1/2] Microsoft Device Code Login                          │
  └─────────────────────────────────────────────────────────────┘
""")
        status, body = _http_post(MS_DEVICE_AUTH, data=urlencode({
            "client_id": MS_CLIENT_ID,
            "scope": MS_SCOPE,
        }))
        if status != 200:
            log.die(f"Device code request failed ({status})", 
                    hint=body.decode(errors="replace")[:500])
        dev_data = json.loads(body)
        user_code = dev_data["user_code"]
        device_code = dev_data["device_code"]
        interval = int(dev_data.get("interval", 5))
        expires_in = int(dev_data.get("expires_in", 900))

        print(f"  ┌──────────────────────────────────────────────────────┐")
        print(f"  │                                                      │")
        print(f"  │   Open:  https://microsoft.com/link                  │")
        print(f"  │                                                      │")
        print(f"  │   Enter this code:  {user_code}                         │")
        print(f"  │                                                      │")
        print(f"  │   Code expires in {expires_in // 60} minutes.                     │")
        print(f"  │                                                      │")
        print(f"  └──────────────────────────────────────────────────────┘")
        print(f"\n  Waiting for you to complete login...")

        deadline = time.time() + expires_in
        while time.time() < deadline:
            time.sleep(interval)
            status, body = _http_post(MS_DEVICE_TOKEN, data=urlencode({
                "client_id": MS_CLIENT_ID,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "device_code": device_code,
            }))
            if status == 200:
                ms_data = json.loads(body)
                self.access_token  = ms_data["access_token"]
                self.refresh_token = ms_data["refresh_token"]
                self.expires_at    = time.time() + ms_data.get("expires_in", 3600)
                break
            err = json.loads(body) if body else {}
            if err.get("error") == "authorization_pending":
                print(f"\r  Waiting... ({(deadline - time.time()):.0f}s left)", end="", flush=True)
                continue
            if err.get("error") == "slow_down":
                interval += 5
                continue

            err_desc = err.get('error_description', body.decode(errors='replace')[:300])
            log.die(f"Device login failed: {err_desc}")
        else:
            log.die("Timed out waiting for device code login.")

        log.success("Authenticated with Microsoft!")

        log.step(2, 2, "Completing Minecraft authentication...")
        self.mc_token, self.username, self.uuid, self.expires_at = \
            self.do_full_auth_chain(self.access_token)

        log.success(f"Logged in as: {self.username} ({self.uuid})")
        return True

    def do_full_auth_chain(self, ms_access_token):

        # Xbox Live RPS sometimes requires the "d=" prefix and sometimes rejects it.
        # Try both variants before giving up.
        for ticket in (ms_access_token, f"d={ms_access_token}"):
            status, body = _http_post(XBL_AUTH_URL, json_data={
                "Properties": {
                    "AuthMethod": "RPS",
                    "SiteName": "user.auth.xboxlive.com",
                    "RpsTicket": ticket,
                },
                "RelyingParty": "http://auth.xboxlive.com",
                "TokenType": "JWT",
            })
            if status == 200:
                break
        if status != 200:
            log.die(f"Xbox Live auth failed ({status})",
                    hint=body.decode(errors="replace")[:300])
        xbl = json.loads(body)
        xbl_token = xbl["Token"]
        uhs = xbl["DisplayClaims"]["xui"][0]["uhs"]

        status, body = _http_post(XSTS_AUTH_URL, json_data={
            "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl_token]},
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        })
        if status != 200:
            err = json.loads(body) if body else {}
            xerr = err.get("XErr", "?")
            err_msg = {
                2148916233: "No Xbox Live profile. Create one at https://www.xbox.com/",
                2148916235: "Xbox Live is not available in your country/region.",
                2148916236: "Adult verification required (South Korea age-gating).",
                2148916237: "Adult verification required (South Korea age-gating).",
                2148916238: "Child account — must be added to an Xbox Family by an adult.",
            }.get(xerr, f"XSTS error {xerr}")
            log.die(f"XSTS auth failed: {err_msg}")
        xsts = json.loads(body)
        xsts_token = xsts["Token"]
        xsts_uhs   = xsts["DisplayClaims"]["xui"][0]["uhs"]

        status, body = _http_post(MC_LOGIN_URL, json_data={
            "identityToken": f"XBL3.0 x={xsts_uhs};{xsts_token}",
        })
        if status != 200:
            log.die(f"Minecraft login failed ({status})", 
                    hint=body.decode(errors="replace")[:300])
        mc_auth = json.loads(body)
        mc_token = mc_auth["access_token"]

        expires_at = time.time() + mc_auth.get("expires_in", 86400)

        status, body = _http_get(MC_PROFILE_URL, headers={
            "Authorization": f"Bearer {mc_token}"
        })
        if status != 200:
            log.die(f"Minecraft profile fetch failed ({status})", 
                    hint=body.decode(errors="replace")[:300])
        profile = json.loads(body)
        return mc_token, profile["name"], profile["id"], expires_at

    def try_refresh(self, refresh_token):

        status, body = _http_post(MS_TOKEN_URL, data=urlencode({
            "client_id": MS_CLIENT_ID,
            "refresh_token": refresh_token,
            "grant_type": "refresh_token",
            "scope": MS_SCOPE,
        }))
        if status != 200:
            log.warn(f"Microsoft token refresh failed ({status})")
            return False
        data = json.loads(body)
        self.access_token  = data["access_token"]
        self.refresh_token = data.get("refresh_token", refresh_token)
        self.expires_at    = time.time() + data.get("expires_in", 3600)

        try:
            self.mc_token, self.username, self.uuid, self.expires_at = \
                self.do_full_auth_chain(self.access_token)
            return True
        except SystemExit:
            return False

class VersionManager:
    def __init__(self, game_dir: Path):
        self.game_dir = game_dir
        self.versions_dir = game_dir / "versions"
        self.libraries_dir = game_dir / "libraries"
        self.assets_dir = game_dir / "assets"

    def fetch_manifest(self):
        manifest_path = self.game_dir / "version_manifest_v2.json"

        if manifest_path.exists():
            if time.time() - manifest_path.stat().st_mtime < 300:
                try:
                    return json.loads(manifest_path.read_text(encoding="utf-8"))
                except Exception:
                    pass

        status, body = _http_get(MC_MANIFEST)
        if status != 200:
            log.die(f"Cannot fetch version manifest ({status})", 
                    hint=body.decode(errors="replace")[:300])

        self.game_dir.mkdir(parents=True, exist_ok=True)
        manifest_path.write_bytes(body)
        return json.loads(body)

    def get_version_info(self, version_id=None):
        manifest = self.fetch_manifest()
        if version_id is None or version_id == "latest":
            version_id = manifest["latest"]["release"]
        elif version_id == "latest-snapshot":
            version_id = manifest["latest"]["snapshot"]

        entry = None
        for v in manifest["versions"]:
            if v["id"] == version_id:
                entry = v
                break
        if not entry:
            avail = ', '.join(v['id'] for v in manifest['versions'][:15])
            hint = f"Available (Mojang): {avail}..."

            installed = ModManager.list_installed_versions(self.game_dir)
            if installed:
                hint += f"\n         Installed locally: {', '.join(installed)}"
            log.die(f"Version '{version_id}' not found.", hint=hint)

        json_path = self.versions_dir / version_id / f"{version_id}.json"
        if not json_path.exists():
            log.info(f"Downloading version manifest for {version_id}...")
            _download_file(entry["url"], json_path, f"{version_id}.json")

        version_data = json.loads(json_path.read_text(encoding="utf-8"))
        return version_id, version_data

    def download_client_jar(self, version_id, version_data):
        jar_path = self.versions_dir / version_id / f"{version_id}.jar"
        if jar_path.exists():
            return jar_path
        url = version_data["downloads"]["client"]["url"]
        sha1 = version_data["downloads"]["client"]["sha1"]
        log.info(f"Downloading client jar for {version_id}...")
        _download_file(url, jar_path, f"{version_id}.jar", sha1=sha1)
        return jar_path

    def needs_natives(self, lib_name, classifiers):
        osn = os_name()
        for c in classifiers:
            c = c.lower()
            if osn == "windows" and "windows" in c:

                if "64" in c and os_arch() == "x86_64":
                    return c
                if "arm64" in c and os_arch() == "arm64":
                    return c
                if "64" not in c and "arm64" not in c and "32" not in c:
                    return c
                if "32" in c and os_arch() != "x86_64":

                    pass
            elif osn == "linux" and "linux" in c:
                if os_arch() == "arm64" and "arm64" in c:
                    return c
                if os_arch() == "x86_64" and "arm64" not in c and "32" not in c:
                    return c
            elif osn == "osx" and ("osx" in c or "macos" in c):
                if os_arch() == "arm64" and "arm64" in c:
                    return c
                if os_arch() == "x86_64" and "arm64" not in c:
                    return c
        return None

    def download_libraries(self, version_data, natives_dir, max_workers=4):
        libs = version_data.get("libraries", [])
        osn = os_name()
        all_downloads = []
        all_jars = []

        for i, lib in enumerate(libs):
            name = lib["name"]
            parts = name.split(":")
            group, artifact, version = parts[0], parts[1], parts[2]
            classifier = parts[3] if len(parts) > 3 else None

            group_path = group.replace(".", "/")
            lib_dir = self.libraries_dir / group_path / artifact / version
            jar_name = f"{artifact}-{version}"
            if classifier:
                jar_name += f"-{classifier}"
            jar_name += ".jar"

            if "rules" in lib and not self._rules_allow(lib["rules"], default=True):
                continue

            is_native_by_name = classifier and "natives-" in classifier
            label_suffix = " [native]" if is_native_by_name else ""

            if "downloads" in lib and "artifact" in lib["downloads"]:
                info = lib["downloads"]["artifact"]
                jar_path = lib_dir / jar_name
                if not jar_path.exists():
                    all_downloads.append((info["url"], jar_path,
                                         f"{group}:{artifact}{label_suffix}"))
                all_jars.append(jar_path)
            else:
                jar_path = lib_dir / jar_name
                if not jar_path.exists():
                    url = (f"https://libraries.minecraft.net/{group_path}/{artifact}/"
                           f"{version}/{jar_name}")
                    all_downloads.append((url, jar_path,
                                         f"{group}:{artifact}{label_suffix}"))
                all_jars.append(jar_path)

            if "natives" in lib:
                native_key = lib["natives"].get(osn)
                if native_key:
                    native_key = native_key.replace("${arch}", os_arch())
                    if "downloads" in lib and "classifiers" in lib["downloads"]:
                        classifiers = lib["downloads"]["classifiers"]
                        match_key = self.needs_natives(name, classifiers.keys())
                        if match_key and match_key in classifiers:
                            nc = classifiers[match_key]
                            native_jar = lib_dir / f"{artifact}-{version}-{match_key}.jar"
                            if not native_jar.exists():
                                all_downloads.append((nc["url"], native_jar,
                                                     f"{group}:{artifact} [native]"))

        new_count = len(all_downloads)
        log.info(f"Libraries: {len(all_jars)} total, {new_count} to download [{max_workers} threads]...")

        if all_downloads:
            dl = [0]
            lock = threading.Lock()

            def _dl(task):
                url, dest, label = task
                if dest.exists():
                    return True
                try:
                    _download_file(url, dest, label, show_progress=False)
                    return True
                except Exception:
                    return False

            with ThreadPoolExecutor(max_workers=max_workers) as pool:
                futures = {pool.submit(_dl, t): t for t in all_downloads}
                for f in as_completed(futures):
                    _, dest, label = futures[f]
                    ok = f.result()
                    with lock:
                        dl[0] += 1
                        pct = dl[0] * 100 // len(all_downloads)
                        filled = pct * 25 // 100
                        bar = "\u2588" * filled + "\u2591" * (25 - filled)
                        print(f"\r  [{pct:3d}%] {bar} {dl[0]}/{len(all_downloads)} libs", end="", flush=True)
                    if not ok:
                        print(f"\n    FAILED: {label}")

            bar_done = "\u2588" * 25
            print(f"\r  [100%] {bar_done} {len(all_downloads)}/{len(all_downloads)} libs \u2014 done    ")

        for _, dest, label in all_downloads:
            if ("[native]" in label or "natives-" in label) and dest.exists():
                self._extract_natives(dest, natives_dir)

        seen = {d for _, d, _ in all_downloads}
        for lib in libs:
            name = lib.get("name", "")
            if "rules" in lib and not self._rules_allow(lib["rules"], default=True):
                continue

            parts = name.split(":")
            classifier = parts[3] if len(parts) > 3 else None
            is_native_new = classifier and "natives-" in classifier
            group, artifact, version = parts[0], parts[1], parts[2]
            group_path = group.replace(".", "/")
            lib_dir = self.libraries_dir / group_path / artifact / version

            if is_native_new:
                jar_name_native = f"{artifact}-{version}-{classifier}.jar"
                cached_jar = lib_dir / jar_name_native
                if cached_jar not in seen and cached_jar.exists():
                    self._extract_natives(cached_jar, natives_dir)
                    seen.add(cached_jar)

            if "natives" in lib:
                native_key = lib["natives"].get(osn)
                if native_key:
                    native_key = native_key.replace("${arch}", os_arch())
                    if "downloads" in lib and "classifiers" in lib["downloads"]:
                        classifiers = lib["downloads"]["classifiers"]
                        match_key = self.needs_natives(name, classifiers.keys())
                        if match_key and match_key in classifiers:
                            cached_jar = lib_dir / f"{artifact}-{version}-{match_key}.jar"
                            if cached_jar not in seen and cached_jar.exists():
                                self._extract_natives(cached_jar, natives_dir)
                                seen.add(cached_jar)

        return all_jars

    @staticmethod
    def _rules_allow(rules, default=False):
        allowed = default
        for rule in rules:
            action = rule.get("action", "allow")
            if "os" in rule:
                os_rule = rule["os"]
                os_match = True
                if "name" in os_rule:
                    os_match = os_match and (os_name() == os_rule["name"])
                if "arch" in os_rule:
                    os_match = os_match and (os_arch() == os_rule["arch"])
                if os_match:
                    allowed = (action == "allow")
            elif "features" in rule:

                pass
            else:

                allowed = (action == "allow")
        return allowed

    def _extract_natives(self, jar_path, natives_dir):
        natives_dir.mkdir(parents=True, exist_ok=True)
        try:
            with zipfile.ZipFile(jar_path, "r") as z:
                for entry in z.namelist():
                    name = entry.split("/")[-1]
                    if not name:
                        continue
                    ext = name.rsplit(".", 1)[-1].lower() if "." in name else ""
                    if ext in ("dll", "so", "dylib", "jnilib"):
                        target = natives_dir / name
                        if not target.exists():
                            with z.open(entry) as src:
                                target.write_bytes(src.read())
        except (zipfile.BadZipFile, OSError) as e:
            log.warn(f"Failed to extract natives from {jar_path.name}: {e}")

    def download_assets(self, version_data, max_workers=4):
        asset_index_info = version_data.get("assetIndex", {})
        if not asset_index_info:
            return version_data.get("assets", "legacy")

        index_id = asset_index_info["id"]
        index_path = self.assets_dir / "indexes" / f"{index_id}.json"

        if not index_path.exists():
            _download_file(asset_index_info["url"], index_path, f"Asset index {index_id}")

        index_data = json.loads(index_path.read_text(encoding="utf-8"))
        objects = index_data.get("objects", {})
        total = len(objects)

        missing = []
        for name, obj in objects.items():
            h = obj["hash"]
            sub_dir = h[:2]
            obj_path = self.assets_dir / "objects" / sub_dir / h
            if not obj_path.exists():
                url = f"https://resources.download.minecraft.net/{sub_dir}/{h}"
                missing.append((url, obj_path))

        if not missing:
            log.info(f"Assets: all {total} up to date.")
            return index_id

        log.info(f"Assets: {len(missing)}/{total} to download [{max_workers} threads]...")

        dl = [0]
        fail = [0]
        lock = threading.Lock()

        def _fetch(url, dest):
            try:
                _download_file(url, dest, "", show_progress=False)
                return True
            except Exception:
                return False

        with ThreadPoolExecutor(max_workers=max_workers) as pool:
            futures = {pool.submit(_fetch, url, dest): (url, dest) for url, dest in missing}
            for f in as_completed(futures):
                with lock:
                    if f.result():
                        dl[0] += 1
                    else:
                        fail[0] += 1
                    done = dl[0] + fail[0]
                    pct = done * 100 // len(missing)
                    filled = pct * 25 // 100
                    bar = "\u2588" * filled + "\u2591" * (25 - filled)
                    print(f"\r  [{pct:3d}%] {bar} {done}/{len(missing)} assets", end="", flush=True)

        bar_done = "\u2588" * 25
        print(f"\r  [100%] {bar_done} {dl[0]} new, {fail[0]} failed \u2014 done    ")
        return index_id

class MinecraftLauncher:
    def __init__(self, game_dir: Path, threads: int = 4):
        self.game_dir = game_dir
        self.accounts  = AccountManager(game_dir)
        self.versions  = VersionManager(game_dir)
        self._java      = None
        self.threads   = max(1, min(threads, 32))

    @property
    def java(self):
        if self._java is None:
            self._java = check_java()
        return self._java

    def _select_java(self, version_data):
        """Pick a Java matching the version's required major (e.g. 21 for 1.21.x).
        Forge/NeoForge toolchains (Mixin/ASM) break on newer Java, so an exact
        major match is preferred; falls back to Mojang's bundled runtime."""
        jv = version_data.get("javaVersion", {}) or {}
        required = jv.get("majorVersion")
        component = jv.get("component", "java-runtime-gamma")

        if self._java is not None:
            major = _java_major(self._java)
            if required and major and major != required:
                log.warn(f"Specified Java is version {major}, but Minecraft "
                         f"{version_data.get('id', '?')} expects Java {required}. "
                         f"Mods (Forge/NeoForge) may crash.")
            return self._java

        if not required:
            return self.java

        java = check_java(required_major=required)
        if java:
            self._java = java
            return java

        exe_name = "java.exe" if platform.system() == "Windows" else "java"
        cached = sorted((self.game_dir / "java" / component).glob(f"**/bin/{exe_name}"))
        if cached:
            self._java = str(cached[0])
            return self._java

        log.info(f"No local Java {required} found; fetching Mojang Java runtime...")
        java = download_mojang_java(self.game_dir, component, self.threads)
        if java:
            self._java = java
            return java

        log.warn(f"Could not get Java {required}; falling back to system Java. "
                 f"Modded launches may crash (e.g. Mixin errors on newer Java).")
        return self.java

    def _load_loader_profile(self, mc_version, loader=None, use_fabric=False):
        if loader is None and use_fabric:
            loader = "fabric"
        if loader:
            path = self.game_dir / "libraries" / loader / f"{loader}-profile-{mc_version}.json"
            if path.exists():
                return loader, json.loads(path.read_text(encoding="utf-8"))
            install_cmd = ("install-fabric" if loader == "fabric"
                           else f"install-{loader}")
            log.die(f"--{loader} requested but no {loader} profile found for {mc_version}.",
                    hint=f"Install it first: python mc_launcher.py {install_cmd} -v {mc_version}")

        for candidate in ("fabric", "neoforge", "forge"):
            path = self.game_dir / "libraries" / candidate / f"{candidate}-profile-{mc_version}.json"
            if path.exists():
                return candidate, json.loads(path.read_text(encoding="utf-8"))
        return None, None

    def launch(self, version_id=None, account_data=None, ram_mb=4096,
               loader=None, use_fabric=False, width=None, height=None):
        if account_data is None:
            account_data = self.accounts.get_default()
        if account_data is None:
            log.die("No account. Run 'login' or 'offline <name>' first.")

        acc_type = account_data["type"]
        username = account_data["username"]
        user_uuid = account_data["uuid"]

        if acc_type == "msa":
            if time.time() > account_data.get("expires_at", 0):
                log.info("Session expired. Attempting silent token refresh...")
                auth = MicrosoftAuth()
                ok = auth.try_refresh(account_data["refresh_token"])
                if not ok:
                    log.die("Token refresh failed. Run 'login' again to re-authenticate.")

                uid = auth.uuid
                if len(uid) == 32:
                    uid = f"{uid[0:8]}-{uid[8:12]}-{uid[12:16]}-{uid[16:20]}-{uid[20:32]}"
                self.accounts.set_msa(auth.username, uid, auth.mc_token,
                                      auth.refresh_token, auth.expires_at)
                account_data = self.accounts.get_default()
                log.success(f"Token refreshed — {auth.username}")
            access_token = account_data.get("access_token", "0")
        else:
            access_token = "0"

        version_id, version_data = self.versions.get_version_info(version_id)
        mc_version = version_id

        version_game_dir = self.game_dir / "versions" / mc_version
        version_game_dir.mkdir(parents=True, exist_ok=True)

        loader, loader_profile = self._load_loader_profile(mc_version, loader, use_fabric)
        if loader:
            log.header(f"Minecraft {mc_version} ({loader.title()}) | {username} ({acc_type})")
        else:
            log.header(f"Minecraft {mc_version} | {username} ({acc_type})")
        log.info(f"Game dir: {version_game_dir}\n")

        log.step(1, 4, "Downloading client jar...")
        client_jar = self.versions.download_client_jar(mc_version, version_data)

        natives_dir = self.game_dir / "natives" / mc_version
        natives_dir.mkdir(parents=True, exist_ok=True)

        log.step(2, 4, "Downloading libraries...")
        lib_jars = self.versions.download_libraries(version_data, natives_dir, self.threads)

        log.step(3, 4, "Downloading assets...")
        assets_index = self.versions.download_assets(version_data, self.threads)

        log.step(4, 4, "Launching game...")

        sep = ";" if platform.system() == "Windows" else ":"

        extra_cp = []

        if loader_profile:
            loader_label = loader.title()
            for lib in loader_profile.get("libraries", []):
                if "rules" in lib and not self.versions._rules_allow(lib["rules"], default=True):
                    continue
                name = lib["name"]
                rel_path = None
                if "downloads" in lib and "artifact" in lib["downloads"]:
                    rel_path = lib["downloads"]["artifact"].get("path")
                if not rel_path:
                    rel_path = _maven_rel_path(name)
                lib_jar = self.game_dir / "libraries" / rel_path
                if not lib_jar.exists():
                    url = None
                    if "downloads" in lib and "artifact" in lib["downloads"]:
                        url = lib["downloads"]["artifact"].get("url")
                    elif lib.get("url"):
                        url = f"{lib['url'].rstrip('/')}/{rel_path}"
                    if url:
                        try:
                            _download_file(url, lib_jar, lib_jar.name, show_progress=False)
                        except Exception as e:
                            log.warn(f"Could not download loader library {lib_jar.name}: {e}")
                if lib_jar.exists():
                    extra_cp.append(str(lib_jar))
                else:
                    log.warn(f"Loader library missing: {lib_jar}")
            print(f"  {loader_label}:  {len(extra_cp)} lib jars")

            mods_dir = ModManager(self.game_dir)._mods_dir(mc_version)
            mod_jars = sorted(f for f in mods_dir.iterdir()
                              if f.name.endswith(".jar")
                              and not f.name.endswith(".disabled")
                              and not f.name.endswith("-sources.jar")
                              and not f.name.endswith("-javadoc.jar"))
            if mod_jars:
                # Forge/NeoForge (BootstrapLauncher) must NOT have mod jars on the
                # classpath: they would be turned into boot-layer modules and FML
                # would then skip them when scanning the mods folder. Both loaders
                # discover mods from <gameDir>/mods on their own.
                if loader == "fabric":
                    extra_cp.extend(str(p) for p in mod_jars)
                print(f"  Mods:    {len(mod_jars)} jar(s)")

        all_cp = [client_jar] + lib_jars + extra_cp

        seen = set()
        deduped_cp = []
        for p in all_cp:
            key = str(p)
            if key not in seen:
                seen.add(key)
                deduped_cp.append(p)
        if len(deduped_cp) < len(all_cp):
            log.debug(f"Removed {len(all_cp) - len(deduped_cp)} duplicate classpath entry(ies)")
        all_cp = deduped_cp
        classpath = sep.join(str(p) for p in all_cp)

        if loader_profile:
            main_class = loader_profile.get("mainClass")
            if not main_class:
                log.die(f"Loader profile for {loader} has no mainClass.")
        else:
            main_class = version_data.get("mainClass", "net.minecraft.client.main.Main")

        jvm_args = []

        if "arguments" in version_data:
            for arg in version_data["arguments"].get("jvm", []):
                if isinstance(arg, str):
                    jvm_args.append(arg)
                elif isinstance(arg, dict) and self.versions._rules_allow(arg.get("rules", [])):
                    val = arg.get("value")
                    if isinstance(val, list):
                        jvm_args.extend(val)
                    elif isinstance(val, str):
                        jvm_args.append(val)
        else:

            jvm_args = [
                "-Djava.library.path=${natives_directory}",
                "-cp", "${classpath}",
            ]

        if not any(a.startswith("-Xmx") or a.startswith("-Xms") for a in jvm_args):
            jvm_args = [f"-Xmx{ram_mb}M", f"-Xms{min(1024, ram_mb // 2)}M"] + jvm_args
        if not any("natives" in a.lower() for a in jvm_args):
            jvm_args.append(f"-Djava.library.path={natives_dir}")

        if loader_profile:
            for arg in loader_profile.get("arguments", {}).get("jvm", []):
                if isinstance(arg, str):
                    jvm_args.append(arg)
                elif isinstance(arg, dict) and self.versions._rules_allow(arg.get("rules", [])):
                    val = arg.get("value")
                    if isinstance(val, list):
                        jvm_args.extend(val)
                    elif isinstance(val, str):
                        jvm_args.append(val)

        game_args = []
        if "arguments" in version_data:
            for arg in version_data["arguments"].get("game", []):
                if isinstance(arg, str):
                    game_args.append(arg)
                elif isinstance(arg, dict) and self.versions._rules_allow(arg.get("rules", [])):
                    val = arg.get("value")
                    if isinstance(val, list):
                        game_args.extend(val)
                    elif isinstance(val, str):
                        game_args.append(val)
        elif "minecraftArguments" in version_data:
            game_args = version_data["minecraftArguments"].split(" ")

        if loader_profile:
            for arg in loader_profile.get("arguments", {}).get("game", []):
                if isinstance(arg, str):
                    game_args.append(arg)
                elif isinstance(arg, dict) and self.versions._rules_allow(arg.get("rules", [])):
                    val = arg.get("value")
                    if isinstance(val, list):
                        game_args.extend(val)
                    elif isinstance(val, str):
                        game_args.append(val)

        replacements = {
            "${auth_player_name}":  username,
            "${auth_uuid}":         user_uuid.replace("-", ""),
            "${auth_access_token}": access_token,
            "${auth_session}":      access_token,
            "${clientid}":          "0",
            "${xuid}":              "0",
            "${auth_xuid}":         "0",
            "${user_type}":         "msa" if acc_type == "msa" else "legacy",
            "${user_properties}":   "{}",
            "${version_name}":      version_id,
            "${version_type}":      version_data.get("type", "release"),
            "${game_directory}":    str(version_game_dir),
            "${game_assets}":       str(self.assets_dir),
            "${assets_root}":       str(self.assets_dir),
            "${assets_index_name}": assets_index,
            "${launcher_name}":     LAUNCHER_NAME,
            "${launcher_version}":  LAUNCHER_VER,
            "${classpath_separator}": sep,
            "${classpath}":         classpath,
            "${natives_directory}": str(natives_dir),
            "${library_directory}": str(self.versions.libraries_dir),
            "${resolution_width}":  str(width) if width else "854",
            "${resolution_height}": str(height) if height else "480",
        }

        def replace_tokens(args_list):
            result = []
            for a in args_list:
                for token, val in replacements.items():
                    a = a.replace(token, val)
                result.append(a)
            return result

        jvm_args = replace_tokens(jvm_args)
        game_args = replace_tokens(game_args)

        if not any("-cp" in a or "-classpath" in a for a in jvm_args):
            jvm_args = jvm_args + ["-cp", classpath]

        java = self._select_java(version_data)
        java_ver = _java_major(java)
        cmd = [java] + jvm_args + [main_class] + game_args

        log.info(f"Java:    {java}" + (f" (Java {java_ver})" if java_ver else ""))
        log.info(f"Version: {version_id}")
        log.info(f"Player:  {username}")
        log.info(f"RAM:     {ram_mb} MB")
        log.success("Starting Minecraft...\n")

        env = os.environ.copy()
        proc = subprocess.Popen(cmd, cwd=str(version_game_dir), env=env)
        log.success(f"Minecraft PID: {proc.pid}")
        log.info("Waiting for game to close... (Ctrl+C to force quit)\n")
        try:
            proc.wait()
        except KeyboardInterrupt:
            log.warn("Interrupted. Terminating Minecraft...")
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
            print("  Terminated.")
        return proc.returncode

    def download_version(self, version_id=None, skip_assets=False):
        version_id, version_data = self.versions.get_version_info(version_id)

        version_game_dir = self.game_dir / "versions" / version_id
        version_game_dir.mkdir(parents=True, exist_ok=True)

        print(f"\n  Target:  Minecraft {version_id}")
        print(f"  Dir:     {self.game_dir}")
        print(f"  Game:    {version_game_dir}  (version-isolated)\n")

        print("[1/3] Client jar...")
        jar = self.versions.download_client_jar(version_id, version_data)
        jar_mb = jar.stat().st_size / 1024 / 1024
        print(f"       {jar.name}  ({jar_mb:.1f} MB)")

        print("[2/3] Libraries + natives...")
        natives_dir = self.game_dir / "natives" / version_id
        if natives_dir.exists():
            shutil.rmtree(natives_dir, ignore_errors=True)
        natives_dir.mkdir(parents=True, exist_ok=True)
        lib_jars = self.versions.download_libraries(version_data, natives_dir, self.threads)
        lib_count = len(lib_jars)
        lib_mb = sum(
            p.stat().st_size for p in lib_jars if p.exists()
        ) / 1024 / 1024
        print(f"       {lib_count} jars  (~{lib_mb:.1f} MB)")

        if skip_assets:
            print("[3/3] Assets skipped (--no-assets).")
        else:
            print("[3/3] Assets...")
            assets_index = self.versions.download_assets(version_data, self.threads)

        total_mb = jar_mb + lib_mb
        print(f"\n  [OK] Minecraft {version_id} downloaded (~{total_mb:.0f} MB) -> {self.game_dir}")
        print(f"    Game data (saves, mods, etc.) isolated to: {version_game_dir}")

        account = self.accounts.get_default()
        if account:
            print(f"    Account: {account['username']} ({account['type']})")
            print(f"    Launch:  python mc_launcher.py launch -v {version_id}")
        else:
            print(f"    No account saved yet. Login first:")
            print(f"      python mc_launcher.py login")
            print(f"    Then launch:")
            print(f"      python mc_launcher.py launch -v {version_id}")

        for loader in ("fabric", "forge", "neoforge"):
            p = self.game_dir / "libraries" / loader / f"{loader}-profile-{version_id}.json"
            if p.exists():
                print(f"    {loader.title()}:  detected — add --{loader} to launch command")
        return version_id, version_data

    @property
    def assets_dir(self):
        return self.versions.assets_dir

def _parse_ram(value):
    """Parse RAM string like '4G', '2048M', or plain integer in MB."""
    if isinstance(value, int):
        return value
    value = str(value).strip().upper()
    m = re.match(r'^(\d+(?:\.\d+)?)\s*(G|GB|M|MB)?$', value)
    if not m:
        raise argparse.ArgumentTypeError(
            f"Invalid RAM value: '{value}'. Use format like 4G, 2048M, or a plain number in MB.")
    num = float(m.group(1))
    unit = (m.group(2) or "M").upper()
    if unit.startswith("G"):
        return int(num * 1024)
    return int(num)

def main():
    parser = argparse.ArgumentParser(
        description="Simple Minecraft CLI Launcher — Microsoft + offline + Fabric/Forge/NeoForge + Modrinth mods",
        epilog="Examples:\n"
               "  %(prog)s login                       # Microsoft login (browser, default)\n"
               "  %(prog)s login --device-code         # Device code login (requires custom Azure app)\n"
               "  %(prog)s offline Steve               # Offline mode (save credentials)\n"
               "  %(prog)s play                        # Launch with saved account + version\n"
               "  %(prog)s play -v 1.21.4              # Launch specific version\n"
               "  %(prog)s play -v 1.21.4 --ram 4G     # Allocate 4 GB RAM\n"
               "  %(prog)s play --fabric                # Launch with Fabric + mods\n"
               "  %(prog)s play --forge                 # Launch with Forge + mods\n"
               "  %(prog)s play --neoforge              # Launch with NeoForge + mods\n"
               "  %(prog)s accounts                    # Show saved accounts\n"
               "  %(prog)s download                    # Download latest version only\n"
               "  %(prog)s download -v 1.20.1 --no-assets  # Jar+libs only\n"
               "  %(prog)s download --threads 16           # 16-thread download\n"
               "  %(prog)s list-versions               # List all Minecraft versions\n"
               "  %(prog)s list-loaders                # List all mod loaders\n"
               "  %(prog)s search sodium               # Search mods on Modrinth\n"
               "  %(prog)s search-more sodium          # Detailed info (exact slug required)\n"
               "  %(prog)s install-fabric -v 1.21.4    # Install Fabric loader\n"
               "  %(prog)s install-forge -v 1.20.1     # Install Forge loader\n"
               "  %(prog)s install-neoforge -v 1.21.4  # Install NeoForge loader\n"
               "  %(prog)s install-mod sodium -v 1.21.4     # Install a mod\n"
               "  %(prog)s list-installed              # List locally installed versions\n"
               "  %(prog)s list-mods -v 1.21.4         # List mods for a version\n"
               "  %(prog)s disable-mod sodium -v 1.21.4     # Disable a mod\n"
               "  %(prog)s enable-mod sodium -v 1.21.4      # Re-enable a mod\n"
               "  %(prog)s uninstall-mod sodium -v 1.21.4   # Uninstall a mod\n"
               "  %(prog)s logout                      # Clear saved session",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("action", nargs="?", default="play",
                        choices=["login", "offline", "play", "launch", "download",
                                 "logout", "accounts",
                                 "list-versions", "list-loaders", "list-installed",
                                 "list-mods", "search", "search-more",
                                 "install-fabric", "install-forge", "install-neoforge",
                                 "install-mod",
                                 "disable-mod", "enable-mod", "uninstall-mod"],
                        help="Action to perform ('play' and 'launch' are equivalent)")
    parser.add_argument("query", nargs="?", default=None,
                        help="Username (offline mode) or mod search query / mod slug")
    parser.add_argument("--version", "-v", default=None,
                        help="Minecraft version (default: latest release)")
    parser.add_argument("--loader", "-l", default=None,
                        help="Mod loader filter (fabric, forge, neoforge, quilt, etc.)")
    parser.add_argument("--loader-version", default=None,
                        help="Specific loader version ID to install")
    parser.add_argument("--mod-version", default=None,
                        help="Specific mod version ID to install")
    parser.add_argument("--limit", type=int, default=10,
                        help="Max search results (default: 10)")
    parser.add_argument("--dir", "-d", default=str(DEFAULT_DIR),
                        help=f"Game directory (default: {DEFAULT_DIR})")
    parser.add_argument("--ram", "-r", type=_parse_ram, default="4G",
                        help="RAM allocation (default: 4G). Accepts: 4G, 2048M, or plain MB number")
    parser.add_argument("--java", "-j", default=None,
                        help="Path to Java executable (auto-detected if omitted)")
    parser.add_argument("--width", type=int, default=None,
                        help="Game window width in pixels")
    parser.add_argument("--height", type=int, default=None,
                        help="Game window height in pixels")
    parser.add_argument("--no-assets", action="store_true",
                        help="Skip asset downloads (jar + libraries only)")
    parser.add_argument("--threads", "-t", type=int, default=4,
                        help="Parallel download threads (default: 4, max: 32)")
    parser.add_argument("--fabric", action="store_true",
                        help="Launch with Fabric loader (auto-detected if installed)")
    parser.add_argument("--forge", action="store_true",
                        help="Launch with Forge loader (auto-detected if installed)")
    parser.add_argument("--neoforge", action="store_true",
                        help="Launch with NeoForge loader (auto-detected if installed)")
    parser.add_argument("--browser", action="store_true",
                        help="Use browser login (default; requires URL copy-paste)")
    parser.add_argument("--device-code", action="store_true",
                        help="Use Microsoft device code login (requires a custom Azure app)")

    args = parser.parse_args()
    game_dir = Path(args.dir)

    # Normalize aliases
    if args.action == "launch":
        args.action = "play"

    if args.forge and args.neoforge:
        log.die("Cannot use both --forge and --neoforge.")
    if not args.loader:
        if args.forge:
            args.loader = "forge"
        elif args.neoforge:
            args.loader = "neoforge"
        elif args.fabric:
            args.loader = "fabric"

    launcher = MinecraftLauncher(game_dir, threads=args.threads)

    # Override Java path if specified
    if args.java:
        launcher._java = args.java

    if args.action == "logout":
        launcher.accounts.clear()
        print("  Cleared all saved accounts.")
        return

    if args.action == "accounts":
        log.header("Saved Accounts")
        default_key = launcher.accounts.data.get("default")
        accs = launcher.accounts.data.get("accounts", {})
        if not accs:
            log.warn("No accounts saved.")
            log.info("Login:  python mc_launcher.py login")
            log.info("Offline: python mc_launcher.py offline <username>")
            return
        for key, acc in accs.items():
            is_default = " (default)" if key == default_key else ""
            acc_type = acc.get("type", "?")
            username = acc.get("username", "?")
            print(f"  [{acc_type}] {username}{is_default}")
            if acc_type == "msa":
                expires = acc.get("expires_at", 0)
                if time.time() > expires:
                    print(f"         Session expired — will auto-refresh on next launch")
                else:
                    remaining = expires - time.time()
                    hours = int(remaining // 3600)
                    print(f"         Session valid (~{hours}h remaining)")
        return

    if args.action == "list-versions":
        log.header("Minecraft Versions (from Modrinth)")
        versions = ModrinthAPI.list_game_versions()

        versions.sort(key=lambda v: v.get("date", ""), reverse=True)
        for v in versions:
            marker = " ★" if v.get("major") else ""
            print(f"  {v['version']:<12s} {v['version_type']:<10s} {v.get('date', '')[:10]}{marker}")
        log.info(f"\nTotal: {len(versions)} versions")
        return

    if args.action == "list-loaders":
        log.header("Mod Loaders")
        loaders = ModrinthAPI.list_loaders()
        print("  Modrinth loaders:")
        for l in loaders:
            print(f"  - {l}")
        print("  Built-in loaders:")
        for l in ("fabric", "forge", "neoforge", "quilt"):
            print(f"  - {l}")
        log.info(f"\nTotal: {len(loaders)} loaders from Modrinth + built-in loaders")
        return

    if args.action == "list-installed":
        log.header("Locally Installed Minecraft Versions")
        versions = ModManager.list_installed_versions(game_dir)
        if not versions:
            log.warn("No versions installed.")
            log.info("Download one: python mc_launcher.py download -v <version>")
            return
        for v in versions:
            vdir = game_dir / "versions" / v
            jar = vdir / f"{v}.jar"
            jar_size = ""
            if jar.exists():
                jar_size = f"  ({jar.stat().st_size / 1024 / 1024:.1f} MB)"
            mod_count = len(list((vdir / "mods").glob("*.jar"))) if (vdir / "mods").exists() else 0
            dis_count = len(list((vdir / "mods").glob("*.jar.disabled"))) if (vdir / "mods").exists() else 0
            tags = []
            for loader in ("fabric", "forge", "neoforge"):
                if (game_dir / "libraries" / loader / f"{loader}-profile-{v}.json").exists():
                    tags.append(loader.title())
            if mod_count:
                tags.append(f"{mod_count} mods")
            if dis_count:
                tags.append(f"{dis_count} disabled")
            tag_str = f"  [{', '.join(tags)}]" if tags else ""
            print(f"  {v:<12s}{jar_size}{tag_str}")
        print(f"\n  Total: {len(versions)} version(s)")
        account = launcher.accounts.get_default()
        if account:
            print(f"  Account: {account['username']} ({account['type']})")
        else:
            print(f"  No account saved. Run: python mc_launcher.py login")
        return

    if args.action == "list-mods":
        mc_version = args.version
        if not mc_version:
            installed = ModManager.list_installed_versions(game_dir)
            if not installed:
                log.die("No versions installed. Specify --version.")
            mc_version = installed[-1]
            log.info(f"Auto-detected version: {mc_version}")
        log.header(f"Mods for Minecraft {mc_version}")
        mm = ModManager(game_dir)
        mods = mm.list_mods(mc_version)
        if not mods:
            log.warn(f"No mods installed for {mc_version}.")
            log.info(f"Install: python mc_launcher.py install-mod <slug> -v {mc_version}")
            return
        for name, enabled, size in mods:
            status = "[enabled] " if enabled else "[DISABLED]"
            print(f"  {status} {name:<50s} {size/1024:>8.1f} KB")
        enabled_count = sum(1 for _, e, _ in mods if e)
        disabled_count = sum(1 for _, e, _ in mods if not e)
        log.info(f"\n{enabled_count} enabled, {disabled_count} disabled")
        return

    if args.action == "disable-mod":
        slug = args.query
        mc_version = args.version
        if not slug:
            log.die("Provide a mod slug/name to disable.")
        if not mc_version:
            installed = ModManager.list_installed_versions(game_dir)
            if not installed:
                log.die("No versions installed. Specify --version.")
            mc_version = installed[-1]
            log.info(f"Auto-detected version: {mc_version}")
        mm = ModManager(game_dir)
        ok = mm.disable_mod(slug, mc_version)
        if ok:
            log.success(f"Disabled '{slug}' for Minecraft {mc_version}")
        else:
            log.die(f"No mod matching '{slug}' found for {mc_version}")
        return

    if args.action == "enable-mod":
        slug = args.query
        mc_version = args.version
        if not slug:
            log.die("Provide a mod slug/name to enable.")
        if not mc_version:
            installed = ModManager.list_installed_versions(game_dir)
            if not installed:
                log.die("No versions installed. Specify --version.")
            mc_version = installed[-1]
            log.info(f"Auto-detected version: {mc_version}")
        mm = ModManager(game_dir)
        ok = mm.enable_mod(slug, mc_version)
        if ok:
            log.success(f"Enabled '{slug}' for Minecraft {mc_version}")
        else:
            log.die(f"No disabled mod matching '{slug}' found for {mc_version}")
        return

    if args.action == "uninstall-mod":
        slug = args.query
        mc_version = args.version
        if not slug:
            log.die("Provide a mod slug/name to uninstall.")
        if not mc_version:
            installed = ModManager.list_installed_versions(game_dir)
            if not installed:
                log.die("No versions installed. Specify --version.")
            mc_version = installed[-1]
            log.info(f"Auto-detected version: {mc_version}")
        mm = ModManager(game_dir)
        deleted = mm.uninstall_mod(slug, mc_version)
        if deleted:
            for d in deleted:
                log.success(f"Deleted: {d}")
        else:
            log.die(f"No mod matching '{slug}' found for {mc_version}")
        return

    if args.action == "search":
        query = args.query
        if not query:
            log.die("Please provide a search query.\n  Example: python mc_launcher.py search sodium")
        log.header(f"Searching for: {query} (source: modrinth)")
        mm = ModManager(game_dir)
        hits = mm.search(query, limit=args.limit,
                         game_version=args.version, loader=args.loader)
        if not hits:
            log.warn("No results found.")
            return

        support_map = {}
        with ThreadPoolExecutor(max_workers=min(8, len(hits))) as pool:
            futures = {pool.submit(mm.loader_support, h["project_id"]): h["project_id"]
                       for h in hits if h.get("project_id")}
            for f in as_completed(futures):
                try:
                    support_map[futures[f]] = f.result()
                except Exception:
                    support_map[futures[f]] = {}

        for i, h in enumerate(hits):
            title = h.get("title", h.get("slug", "?"))
            desc = h.get("description", "")[:100]
            author = h.get("author", "?")
            downloads = h.get("downloads", 0)
            categories = ", ".join(h.get("categories", []))
            source = h.get("source", "?")
            print(f"  [{i+1}] {title}")
            print(f"      slug: {h.get('slug', '?')}  |  source: {source}")
            print(f"      by: {author}  |  downloads: {downloads:,}")
            print(f"      categories: {categories}")
            support = support_map.get(h.get("project_id"), {})
            if support:
                print(f"      support: {ModManager.format_loader_support(support)}")
            if desc:
                print(f"      {desc}...")
            print()
        log.info(f"Found {len(hits)} result(s).")
        log.info("Install with: python mc_launcher.py install-mod <slug> -v <version>")
        log.info("Details with: python mc_launcher.py search-more <slug>")
        return

    if args.action == "search-more":
        slug = args.query
        if not slug:
            log.die("Please provide the exact mod slug.\n  Example: python mc_launcher.py search-more sodium")
        mm = ModManager(game_dir)
        try:
            project = mm.modrinth.get_project(slug)
        except SystemExit:
            log.die(f"Project '{slug}' not found. search-more requires the EXACT slug.",
                    hint=f"Find the slug first: python mc_launcher.py search {slug}")

        versions = mm.modrinth.get_versions(project["id"])
        support = ModManager.summarize_loader_support(versions)

        title = project.get("title", slug)
        log.header(f"{title}")
        print(f"  slug:        {project.get('slug', '?')}")
        print(f"  project id:  {project.get('id', '?')}")
        print(f"  downloads:   {project.get('downloads', 0):,}")
        print(f"  followers:   {project.get('followers', 0):,}")
        print(f"  client/server: {project.get('client_side', '?')} / {project.get('server_side', '?')}")
        lic = project.get("license") or {}
        print(f"  license:     {lic.get('id', '?')}")
        print(f"  categories:  {', '.join(project.get('categories', []))}")
        print(f"  updated:     {project.get('updated', '?')[:10]}")
        if project.get("source_url"):
            print(f"  source:      {project['source_url']}")
        if project.get("issues_url"):
            print(f"  issues:      {project['issues_url']}")
        desc = project.get("description", "")
        if desc:
            print(f"\n  {desc}")

        print(f"\n  Loader support (highest game version):")
        for l in ModManager.LOADERS_SHOWN:
            if l in support:
                mc, modver = support[l]
                print(f"    {l:<10s} <= MC {mc:<10s} latest mod version: {modver}")
            else:
                print(f"    {l:<10s} not supported")
        for l, (mc, modver) in sorted(support.items()):
            if l not in ModManager.LOADERS_SHOWN:
                print(f"    {l:<10s} <= MC {mc:<10s} latest mod version: {modver}")

        all_mc = {g for v in versions for g in v.get("game_versions", [])
                  if ModManager._RELEASE_MC_RE.match(g)}
        if all_mc:
            mc_sorted = sorted(all_mc, key=ModManager._mc_key)
            shown = ", ".join(mc_sorted[-12:])
            prefix = "..., " if len(mc_sorted) > 12 else ""
            print(f"\n  Supported MC releases: {prefix}{shown}")

        versions_sorted = sorted(versions, key=lambda v: v.get("date_published", ""),
                                 reverse=True)
        print(f"\n  Recent versions ({min(8, len(versions_sorted))} of {len(versions_sorted)}):")
        for v in versions_sorted[:8]:
            vn = v.get("version_number", v.get("id", "?"))
            gv = ", ".join(v.get("game_versions", ["?"])[-4:])
            ld = ", ".join(v.get("loaders", ["?"]))
            date = v.get("date_published", "?")[:10]
            vtype = v.get("version_type", "?")
            print(f"    {vn:<36s} {vtype:<8s} MC {gv:<24s} [{ld}]  {date}")
        print(f"\n  Install: python mc_launcher.py install-mod {project.get('slug', slug)} -v <version> --loader <loader>")
        return

    if args.action in ("install-fabric", "install-forge", "install-neoforge"):
        mc_version = args.version
        vm = VersionManager(game_dir)
        if not mc_version:
            manifest = vm.fetch_manifest()
            mc_version = manifest["latest"]["release"]
            log.info(f"Using latest Minecraft release: {mc_version}")
        else:
            manifest = vm.fetch_manifest()
            known = {v["id"] for v in manifest.get("versions", [])}
            if mc_version not in known:
                close = [v for v in known if v.startswith(mc_version[:4])]
                close.sort()
                hint = f"Did you mean: {', '.join(close[-8:])}" if close else ""
                log.die(f"Minecraft version '{mc_version}' does not exist.", hint=hint)

    if args.action == "install-fabric":
        log.header("Install Fabric Loader")
        log.info(f"Target MC version: {mc_version}")

        fm = FabricManager(game_dir)
        all_jars, profile = fm.install(mc_version, args.loader_version)

        log.success("Fabric Loader installed successfully!")
        print(f"    MC Version: {mc_version}")
        print(f"    Profile:    {profile.get('id', '?')}")
        print(f"    Libraries:  {len(all_jars)} jars -> {game_dir / 'libraries'}")
        print(f"\n  Launch with: python mc_launcher.py launch -v {mc_version} --fabric")
        return

    if args.action == "install-forge":
        log.header("Install Forge Loader")
        log.info(f"Target MC version: {mc_version}")

        fm = ForgeManager(game_dir)
        installed_id, profile = fm.install(mc_version, args.loader_version)

        log.success("Forge Loader installed successfully!")
        print(f"    MC Version: {mc_version}")
        print(f"    Version ID: {installed_id}")
        print(f"    Main Class: {profile.get('mainClass', '?')}")
        print(f"\n  Launch with: python mc_launcher.py launch -v {mc_version} --forge")
        return

    if args.action == "install-neoforge":
        log.header("Install NeoForge Loader")
        log.info(f"Target MC version: {mc_version}")

        nm = NeoForgeManager(game_dir)
        installed_id, profile = nm.install(mc_version, args.loader_version)

        log.success("NeoForge Loader installed successfully!")
        print(f"    MC Version: {mc_version}")
        print(f"    Version ID: {installed_id}")
        print(f"    Main Class: {profile.get('mainClass', '?')}")
        print(f"\n  Launch with: python mc_launcher.py launch -v {mc_version} --neoforge")
        return

    if args.action == "install-mod":
        slug = args.query
        if not slug:
            log.die("Please provide a mod slug or search first.\n  Example: python mc_launcher.py install-mod sodium -v 1.21.4")
        mc_version = args.version
        if not mc_version:
            installed = ModManager.list_installed_versions(game_dir)
            if not installed:
                log.die("No Minecraft version installed. Specify --version.\n  Download first: python mc_launcher.py download -v 1.21.4")
            mc_version = installed[-1]
            log.info(f"Auto-detected version: {mc_version}")
        log.header(f"Install Mod for MC {mc_version}")

        mm = ModManager(game_dir)
        paths, version_data, project = mm.install(
            slug,
            mc_version=mc_version,
            loader=args.loader,
            version_id=args.mod_version,
        )

        title = project.get("title", slug)
        source = project.get("source", "?")
        log.success(f"{title} installed for Minecraft {mc_version} (source: {source})!")
        used_loader = mm._pick_loader(mc_version, args.loader)
        if used_loader:
            print(f"  Launch with: python mc_launcher.py launch -v {mc_version} --{used_loader}")
        else:
            print(f"  Install a loader first, e.g.: python mc_launcher.py install-fabric -v {mc_version}")
            print(f"  Then launch: python mc_launcher.py launch -v {mc_version} --fabric")
        return

    if args.action == "login":
        if args.device_code:
            auth = MicrosoftAuth()
            auth.device_code_login()
        else:
            log.header("Microsoft Login (Browser)")
            log.info("A browser window will open. Log in with your Microsoft account.")
            log.info("Make sure your Microsoft account owns Minecraft!\n")
            log.info("Tip: use --device-code for device code login (requires a custom Azure app).\n")
            auth = MicrosoftAuth()
            auth.login()

        uid = auth.uuid
        if len(uid) == 32:
            uid = f"{uid[0:8]}-{uid[8:12]}-{uid[12:16]}-{uid[16:20]}-{uid[20:32]}"
        launcher.accounts.set_msa(auth.username, uid, auth.mc_token,
                                  auth.refresh_token, auth.expires_at)

        log.success(f"\n  Logged in as: {auth.username}")
        log.info("Credentials saved. Next steps:")
        log.info(f"  python mc_launcher.py download -v <version>   # download a Minecraft version")
        log.info(f"  python mc_launcher.py launch -v <version>     # launch the game")

    elif args.action == "offline":
        username = args.query or "Steve"
        log.header(f"Offline Mode: {username}")
        launcher.accounts.set_offline(username)
        log.success(f"Offline account saved: {username}")
        log.info("Next steps:")
        log.info(f"  python mc_launcher.py download -v <version>   # download a Minecraft version")
        log.info(f"  python mc_launcher.py launch -v <version>     # launch the game")

    elif args.action == "download":
        log.header("Download Only")
        launcher.download_version(args.version, skip_assets=args.no_assets)

    elif args.action == "play":
        account = launcher.accounts.get_default()
        if not account:
            log.die("No saved account. Run 'login' or 'offline <name>' first.",
                    hint="  python mc_launcher.py login        # Microsoft login\n"
                         "  python mc_launcher.py offline Steve # Offline mode")
        launcher.launch(args.version, account, args.ram,
                        loader=args.loader,
                        width=args.width, height=args.height)

if __name__ == "__main__":
    main()
