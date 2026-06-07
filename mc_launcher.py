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

def _download_file(url: str, dest_path, label: str = "", sha1: str = None,
                   max_retries: int = 3, show_progress: bool = True):
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

    for attempt in range(max_retries):
        try:
            with urlopen(url, timeout=120) as resp:
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
        for f in version_data.get("files", []):
            file_path = dest / f["filename"]
            if not file_path.exists():
                _download_file(f["url"], file_path, label or f["filename"])
            else:
                print(f"  {f['filename']} — cached")
            paths.append(file_path)
        return paths

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

class ModManager:
    def __init__(self, game_dir: Path):
        self.game_dir = game_dir

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

    def search(self, query, limit=10):
        result = ModrinthAPI.search_projects(query, index="relevance", limit=limit)
        return result.get("hits", [])

    def _detect_loaders(self, mc_version):
        found = []
        p = self.game_dir / "libraries" / "fabric" / f"fabric-profile-{mc_version}.json"
        if p.exists():
            found.append("fabric")

        return found

    def install(self, slug, mc_version, loader=None, version_id=None):
        log.info(f"Resolving mod: {slug}...")
        project = ModrinthAPI.get_project(slug)
        proj_title = project.get("title", slug)
        proj_id = project["id"]

        if not loader:
            detected = self._detect_loaders(mc_version)
            if detected:
                loader = detected[0]
                log.info(f"Auto-detected loader: {loader}")

        kwargs = {"game_versions": [mc_version]}
        if loader:
            kwargs["loaders"] = [loader]

        versions = ModrinthAPI.get_project_versions(proj_id, **kwargs)

        if not versions and loader:
            versions = ModrinthAPI.get_project_versions(proj_id, game_versions=[mc_version])

        if not versions:
            extra = f" for MC {mc_version}"
            extra += f" ({loader})" if loader else ""
            if not loader:
                extra += "\n  Hint: install a loader first, or specify --loader manually"
            log.die(f"No versions found for {proj_title}{extra}")

        if version_id:
            target = None
            for v in versions:
                if v["id"] == version_id:
                    target = v
                    break
            if not target:
                log.die(f"Version '{version_id}' not found for {proj_title}")
        else:

            def _sort_key(v):
                loaders = [l.lower() for l in v.get("loaders", [])]
                loader_match = 0 if loader and loader.lower() in loaders else 1
                date = v.get("date_published", "")
                return (loader_match, date)
            versions.sort(key=_sort_key)
            target = versions[0]

        ver_num = target.get("version_number", target["id"])
        mc_str = ", ".join(target.get("game_versions", ["?"]))
        loaders_str = ", ".join(target.get("loaders", ["?"]))
        if loader and loader.lower() not in [l.lower() for l in target.get("loaders", [])]:
            log.warn(f"Note: selected version uses loader '{loaders_str}', not '{loader}'")
        log.info(f"Installing {proj_title} {ver_num} (MC: {mc_str}, Loaders: {loaders_str})...")

        dest_dir = self._mods_dir(mc_version)
        paths = ModrinthAPI.download_version_files(target, dest_dir,
                                                    f"{proj_title} {ver_num}")
        total_size = sum(p.stat().st_size for p in paths if p.exists())
        log.success(f"Installed {len(paths)} file(s) ({total_size / 1024:.1f} KB) -> {dest_dir}")
        return paths, target, project

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

def check_java():
    java = None
    candidates = []

    jh = os.environ.get("JAVA_HOME", "")
    if jh:
        java_bin = "java.exe" if platform.system() == "Windows" else "java"
        je = Path(jh) / "bin" / java_bin
        if je.exists():
            java = str(je)

    if not java:
        java = shutil.which("java")

    if not java and platform.system() == "Windows":
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
                                    candidates.append(str(je))
                            i += 1
                        except OSError:
                            break
            except OSError:
                continue

    if not java and platform.system() == "Windows":
        user_home = Path(os.environ.get("USERPROFILE", Path.home()))
        for d in sorted(user_home.glob("jdk-*"), reverse=True):
            je = d / "bin" / "java.exe"
            if je.exists():
                candidates.append(str(je))

    if not java and platform.system() == "Windows":
        for base in [r"C:\Program Files\Java", r"C:\Program Files\Eclipse Adoptium",
                     r"C:\Program Files\Microsoft", r"C:\Program Files\Eclipse Foundation",
                     r"C:\Program Files (x86)\Java"]:
            bp = Path(base)
            if bp.exists():

                for d in bp.glob("*"):
                    if d.is_dir():
                        je = d / "bin" / "java.exe"
                        if je.exists() and str(je) not in candidates:
                            candidates.append(str(je))

                        for sd in d.glob("*"):
                            if sd.is_dir():
                                je2 = sd / "bin" / "java.exe"
                                if je2.exists() and str(je2) not in candidates:
                                    candidates.append(str(je2))

    if not java and platform.system() != "Windows":
        for base in ["/usr/lib/jvm", "/usr/local/opt", Path.home() / ".sdkman/candidates/java",
                     Path.home() / ".jdks"]:
            bp = Path(base)
            if bp.exists():
                for d in sorted(bp.glob("*"), reverse=True):
                    je = d / "bin" / "java"
                    if je.exists():
                        candidates.append(str(je))

        hb = Path("/usr/local/opt/openjdk/bin/java")
        if hb.exists():
            candidates.append(str(hb))

    if not java and candidates:

        def _key(p):
            nums = re.findall(r'(\d+)', p)
            return tuple(int(n) for n in nums) if nums else (0,)
        candidates.sort(key=_key, reverse=True)
        java = candidates[0]

    if not java:
        log.die("Java not found. Install Java 17+ from https://adoptium.net/",
                hint='If you already installed Java, set JAVA_HOME:\n         PowerShell: $env:JAVA_HOME = "C:\\path\\to\\jdk"')

    try:
        out = subprocess.check_output([java, "-version"], stderr=subprocess.STDOUT, text=True)
        m = re.search(r'version "(\d+)', out)
        if m:
            ver = int(m.group(1))
            if ver < 17:
                log.warn(f"Java {ver} detected. Minecraft 1.18+ needs Java 17+.")
                print(f"  Found at: {java}")
    except Exception:
        pass
    return java

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
            "scope": "XboxLive.signin offline_access",
            "redirect_uri": MS_REDIRECT,
            "prompt": "select_account",
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
            "scope": "XboxLive.signin offline_access",
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

        status, body = _http_post(XBL_AUTH_URL, json_data={
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": f"d={ms_access_token}",
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        })
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
            "scope": "XboxLive.signin offline_access",
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

    def launch(self, version_id=None, account_data=None, ram_mb=4096, use_fabric=False):
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

        version_game_dir = self.game_dir / "versions" / version_id
        version_game_dir.mkdir(parents=True, exist_ok=True)

        log.header(f"Minecraft {version_id} | {username} ({acc_type})")
        log.info(f"Game dir: {version_game_dir}\n")

        log.step(1, 4, "Downloading client jar...")
        client_jar = self.versions.download_client_jar(version_id, version_data)

        natives_dir = self.game_dir / "natives" / version_id
        natives_dir.mkdir(parents=True, exist_ok=True)

        log.step(2, 4, "Downloading libraries...")
        lib_jars = self.versions.download_libraries(version_data, natives_dir, self.threads)

        log.step(3, 4, "Downloading assets...")
        assets_index = self.versions.download_assets(version_data, self.threads)

        log.step(4, 4, "Launching game...")

        sep = ";" if platform.system() == "Windows" else ":"

        extra_cp = []

        fabric_profile = None
        fabric_profile_path = (self.game_dir / "libraries" / "fabric"
                               / f"fabric-profile-{version_id}.json")
        if fabric_profile_path.exists():
            fabric_profile = json.loads(fabric_profile_path.read_text(encoding="utf-8"))
        elif use_fabric:
            log.warn("--fabric set but no Fabric profile found.")
            log.warn(f"Run: python mc_launcher.py install-fabric -v {version_id}")

        if fabric_profile:
            use_fabric = True
            for lib in fabric_profile.get("libraries", []):
                name = lib["name"]
                rel_path = FabricManager._maven_path(name)
                lib_jar = self.game_dir / "libraries" / rel_path
                if lib_jar.exists():
                    extra_cp.append(str(lib_jar))
            print(f"  Fabric:  {len(extra_cp)} lib jars")

        if use_fabric:
            mods_dir = ModManager(self.game_dir)._mods_dir(version_id)
            mod_jars = sorted(f for f in mods_dir.iterdir()
                              if f.name.endswith(".jar") and not f.name.endswith(".disabled"))
            if mod_jars:
                extra_cp.extend(str(p) for p in mod_jars)
                print(f"  Mods:    {len(mod_jars)} jar(s)")

        all_cp = [client_jar] + lib_jars + extra_cp
        classpath = sep.join(str(p) for p in all_cp)

        if fabric_profile:
            main_class = fabric_profile.get("mainClass", "net.fabricmc.loader.impl.launch.knot.KnotClient")
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

        if fabric_profile:
            for arg in fabric_profile.get("arguments", {}).get("jvm", []):
                jvm_args.append(arg)

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
            "${resolution_width}":  "854",
            "${resolution_height}": "480",
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

        cmd = [self.java] + jvm_args + [main_class] + game_args

        log.info(f"Java:    {self.java}")
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

        fabric_path = self.game_dir / "libraries" / "fabric" / f"fabric-profile-{version_id}.json"
        if fabric_path.exists():
            print(f"    Fabric:  detected — add --fabric to launch command")
        return version_id, version_data

    @property
    def assets_dir(self):
        return self.versions.assets_dir

def main():
    parser = argparse.ArgumentParser(
        description="Simple Minecraft CLI Launcher — Microsoft + offline + Modrinth mods",
        epilog="Examples:\n"
               "  %(prog)s login                    # Microsoft login (save credentials)\n"
               "  %(prog)s login --device-code       # Login via device code (simpler, no paste)\n"
               "  %(prog)s offline Steve            # Offline mode (save credentials)\n"
               "  %(prog)s launch                   # Launch with saved account + version\n"
               "  %(prog)s launch -v 1.21.4         # Launch specific version\n"
               "  %(prog)s launch --fabric           # Launch with Fabric + mods\n"
               "  %(prog)s download                 # Download latest version only\n"
               "  %(prog)s download -v 1.20.1 --no-assets  # Jar+libs only\n"
               "  %(prog)s download --threads 16           # 16-thread download\n"
               "  %(prog)s list-versions            # List all Minecraft versions\n"
               "  %(prog)s list-loaders             # List all mod loaders\n"
               "  %(prog)s search sodium            # Search mods on Modrinth\n"
               "  %(prog)s install-fabric -v 1.21.4 # Install Fabric loader\n"
               "  %(prog)s install-mod sodium -v 1.21.4  # Install a mod\n"
               "  %(prog)s list-installed           # List locally installed versions\n"
               "  %(prog)s list-mods -v 1.21.4      # List mods for a version\n"
               "  %(prog)s disable-mod sodium -v 1.21.4  # Disable a mod\n"
               "  %(prog)s enable-mod sodium -v 1.21.4   # Re-enable a mod\n"
               "  %(prog)s uninstall-mod sodium -v 1.21.4  # Uninstall a mod\n"
               "  %(prog)s logout                   # Clear saved session",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("action", nargs="?", default="launch",
                        choices=["login", "offline", "launch", "download", "logout",
                                 "list-versions", "list-loaders", "list-installed",
                                 "list-mods", "search",
                                 "install-fabric", "install-mod",
                                 "disable-mod", "enable-mod", "uninstall-mod"],
                        help="Action to perform")
    parser.add_argument("query", nargs="?", default=None,
                        help="Username (offline mode) or mod search query / mod slug")
    parser.add_argument("--version", "-v", default=None,
                        help="Minecraft version (default: latest release)")
    parser.add_argument("--loader", "-l", default=None,
                        help="Mod loader filter (fabric, forge, quilt, etc.)")
    parser.add_argument("--loader-version", default=None,
                        help="Specific loader version ID to install")
    parser.add_argument("--mod-version", default=None,
                        help="Specific mod version ID to install")
    parser.add_argument("--limit", type=int, default=10,
                        help="Max search results (default: 10)")
    parser.add_argument("--dir", "-d", default=str(DEFAULT_DIR),
                        help=f"Game directory (default: {DEFAULT_DIR})")
    parser.add_argument("--ram", "-r", type=int, default=4096,
                        help="RAM in MB (default: 4096)")
    parser.add_argument("--no-assets", action="store_true",
                        help="Skip asset downloads (jar + libraries only)")
    parser.add_argument("--threads", "-t", type=int, default=4,
                        help="Parallel download threads (default: 4, max: 32)")
    parser.add_argument("--fabric", action="store_true",
                        help="Launch with Fabric loader (auto-detected if installed)")
    parser.add_argument("--device-code", action="store_true",
                        help="Use device code login (no browser copy-paste needed)")

    args = parser.parse_args()
    game_dir = Path(args.dir)

    launcher = MinecraftLauncher(game_dir, threads=args.threads)

    if args.action == "logout":
        launcher.accounts.clear()
        print("  Cleared all saved accounts.")
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
        log.header("Mod Loaders (from Modrinth)")
        loaders = ModrinthAPI.list_loaders()
        for l in loaders:
            print(f"  - {l}")
        log.info(f"\nTotal: {len(loaders)} loaders")
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
            fabric_ok = (game_dir / "libraries" / "fabric" / f"fabric-profile-{v}.json").exists()
            tags = []
            if fabric_ok:
                tags.append("Fabric")
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
        log.header(f"Searching Modrinth for: {query}")
        hits = ModManager(game_dir).search(query, limit=args.limit)
        if not hits:
            log.warn("No results found.")
            return
        for i, h in enumerate(hits):
            title = h.get("title", h.get("slug", "?"))
            desc = h.get("description", "")[:100]
            author = h.get("author", "?")
            downloads = h.get("downloads", 0)
            categories = ", ".join(h.get("categories", []))
            print(f"  [{i+1}] {title}")
            print(f"      slug: {h.get('slug', '?')}")
            print(f"      by: {author}  |  downloads: {downloads:,}")
            print(f"      categories: {categories}")
            if desc:
                print(f"      {desc}...")
            print()
        log.info(f"Found {len(hits)} result(s).")
        log.info("Install with: python mc_launcher.py install-mod <slug> -v <version>")
        return

    if args.action == "install-fabric":
        mc_version = args.version
        if not mc_version:
            mc_version = "latest"

            manifest = VersionManager(game_dir).fetch_manifest()
            mc_version = manifest["latest"]["release"]
            log.info(f"Using latest Minecraft release: {mc_version}")

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
        log.success(f"{title} installed for Minecraft {mc_version}!")
        fabric = (game_dir / "libraries" / "fabric" / f"fabric-profile-{mc_version}.json").exists()
        if fabric:
            print(f"  Launch with: python mc_launcher.py launch -v {mc_version} --fabric")
        else:
            print(f"  Install Fabric first: python mc_launcher.py install-fabric -v {mc_version}")
            print(f"  Then launch: python mc_launcher.py launch -v {mc_version} --fabric")
        return

    if args.action == "login":
        if args.device_code:
            log.header("Microsoft Device Code Login")
            auth = MicrosoftAuth()
            auth.device_code_login()
        else:
            log.header("Microsoft Login")
            log.info("A browser window will open. Log in with your Microsoft account.")
            log.info("Make sure your Microsoft account owns Minecraft!\n")
            log.info("Tip: use --device-code for a simpler experience (no copy-paste).\n")
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

    elif args.action == "launch":
        account = launcher.accounts.get_default()
        if not account:
            log.die("No saved account. Run 'login' or 'offline <name>' first.")
        launcher.launch(args.version, account, args.ram, use_fabric=args.fabric)

if __name__ == "__main__":
    main()
