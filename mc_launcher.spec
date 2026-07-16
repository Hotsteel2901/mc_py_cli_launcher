# -*- mode: python ; coding: utf-8 -*-
import os
import sys

# Allow the CI workflow to control the output binary name via environment variable.
name = os.environ.get("PYINSTALLER_NAME", "mc_launcher")

a = Analysis(
    [os.path.join(SPECPATH, 'mc_launcher.py')],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)

# On Linux, force the frozen executable to use the host's OpenSSL libraries.
# Bundled libssl/libcrypto from the build runner often conflict with the target
# distribution's CA store and TLS stack, which breaks Microsoft login and other
# HTTPS requests on distros like Arch Linux.
if sys.platform.startswith("linux"):
    a.binaries = [
        b for b in a.binaries
        if not (b[0].startswith("libssl.so") or b[0].startswith("libcrypto.so"))
    ]

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name=name,
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
