#!/usr/bin/env python3
"""Exercise the real CLI/Sparkle installer against disposable local bundles.

Usage: python3 scripts/test-cli-update.py [path-to-built-TorroMail.app]
No account files, real keychain items, installed app or production feed are used.
"""
import base64
import functools
import json
import os
import signal
import http.server
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = Path(sys.argv[1] if len(sys.argv) > 1 else
              ROOT / "apps/TorroMailApp/.build/TorroMail.app").resolve()
TOOLS = ROOT / "apps/TorroMailApp/.build/artifacts/sparkle/Sparkle/bin"


def command(args, expected=0, timeout=90):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=timeout)
    if result.returncode != expected:
        raise AssertionError(f"{args}: expected {expected}, got {result.returncode}\n"
                             f"{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def plist(path, changes):
    with path.open("rb") as f:
        data = plistlib.load(f)
    data.update(changes)
    with path.open("wb") as f:
        plistlib.dump(data, f)


def sign(app):
    for tool in (app / "Contents/MacOS/torromail", app / "Contents/MacOS/TorroMailApp",
                 app / "Contents/Helpers/TorroMailUpdater.app"):
        command(["codesign", "--force", "--sign", "-", tool])
    command(["codesign", "--force", "--deep", "--sign", "-", app])


class Server(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


with tempfile.TemporaryDirectory(prefix="torromail-cli-test-") as directory:
    temp = Path(directory)
    # A private fixture-only Sparkle key, generated in memory (never Keychain).
    source = temp / "key.swift"
    source.write_text('''import CryptoKit
import Foundation
let key = Curve25519.Signing.PrivateKey()
let bytes = key.rawRepresentation
try bytes.base64EncodedString().write(toFile: CommandLine.arguments[1], atomically: true, encoding: .utf8)
print(key.publicKey.rawRepresentation.base64EncodedString())
''')
    command(["xcrun", "swiftc", source, "-o", temp / "keygen"])
    keyfile = temp / "key.txt"
    public_key = command([temp / "keygen", keyfile])
    keyfile.chmod(0o600)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0),
        functools.partial(Server, directory=str(temp)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base_url = f"http://127.0.0.1:{server.server_port}"
    defaults_domain = f"com.torromail.cli-test.{uuid.uuid4()}"
    try:
        # Only the updater, CLI and framework are needed. The placeholder app
        # executable has no mail functionality and is never launched by tests.
        template = temp / "Template.app"
        (template / "Contents/MacOS").mkdir(parents=True)
        shutil.copy2(BUNDLE / "Contents/MacOS/torromail", template / "Contents/MacOS/torromail")
        for name in ("Helpers", "Frameworks"):
            shutil.copytree(BUNDLE / "Contents" / name, template / "Contents" / name, symlinks=True)
        pid_file = temp / "fixture.pid"
        app_source = temp / "app.m"
        app_source.write_text(f"""#import <AppKit/AppKit.h>
#include <unistd.h>
int main(void) {{ @autoreleasepool {{
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    [[NSString stringWithFormat:@"%d", getpid()] writeToFile:@{json.dumps(str(pid_file))}
        atomically:YES encoding:NSUTF8StringEncoding error:NULL];
    [NSApp run];
}} return 0; }}
""")
        command(["xcrun", "clang", "-fobjc-arc", "-framework", "AppKit", app_source,
                 "-o", template / "Contents/MacOS/TorroMailApp"])
        info = {"CFBundleIdentifier": "com.torromail.app", "CFBundleExecutable": "TorroMailApp",
                "CFBundleName": "TorroMail", "CFBundlePackageType": "APPL",
                "CFBundleVersion": "1", "CFBundleShortVersionString": "1.0",
                "SUFeedURL": base_url + "/appcast.xml", "SUPublicEDKey": public_key,
                "SUEnableAutomaticChecks": False, "SUDefaultsDomain": defaults_domain}
        with (template / "Contents/Info.plist").open("wb") as f:
            plistlib.dump(info, f)
        sign(template)
        old = temp / "Relocated Apps" / "TorroMail.app"
        shutil.copytree(template, old, symlinks=True)
        # An alias exercises bundled app discovery, paths with spaces and the
        # canonicalized host path passed to the separate updater.
        alias = temp / "torromail"
        alias.symlink_to(old / "Contents/MacOS/torromail")
        command([alias, "update", "--help"])
        command([alias, "update", "--feed-url", "https://example.com"], expected=2)

        appcast = temp / "appcast.xml"
        appcast.write_text('<?xml version="1.0"?><rss version="2.0"><channel/></rss>')
        command([alias, "update", "--check"], expected=4)
        assert plistlib.loads((old / "Contents/Info.plist").read_bytes())["CFBundleVersion"] == "1"

        new = temp / "release" / "TorroMail.app"
        shutil.copytree(template, new, symlinks=True)
        plist(new / "Contents/Info.plist", {"CFBundleVersion": "2", "CFBundleShortVersionString": "2.0"})
        sign(new)
        archive = temp / "update.zip"
        command(["ditto", "-c", "-k", "--keepParent", new, archive])
        signature = command([TOOLS / "sign_update", "--ed-key-file", keyfile, "-p", archive])

        def feed(sig):
            appcast.write_text(f'''<?xml version="1.0"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>
<title>TorroMail 2.0</title><sparkle:version>2</sparkle:version><sparkle:shortVersionString>2.0</sparkle:shortVersionString>
<enclosure url="{base_url}/update.zip" length="{archive.stat().st_size}" type="application/octet-stream" sparkle:edSignature="{sig}"/>
</item></channel></rss>''')

        feed(signature)
        command([alias, "update", "--check"])
        assert plistlib.loads((old / "Contents/Info.plist").read_bytes())["CFBundleVersion"] == "1"
        print("PASS: check-only detects updates without replacing the closed app")
        feed(base64.b64encode(bytes(64)).decode())
        command([alias, "update"], expected=1)
        assert plistlib.loads((old / "Contents/Info.plist").read_bytes())["CFBundleVersion"] == "1"
        print("PASS: invalid Ed25519 signature is rejected; app stays intact")
        # Sparkle's detached installer exits asynchronously after reporting a
        # signature error. Wait for its fixture processes to close before a
        # retry, otherwise its temporary resume-status service is still live.
        quiet = 0
        deadline = time.monotonic() + 15
        while quiet < 3:
            processes = command(["ps", "-axo", "command"])
            active = any(str(temp) in line and ("/Autoupdate " in line or
                         "/Updater.app/Contents/MacOS/Updater" in line)
                         for line in processes.splitlines())
            quiet = 0 if active else quiet + 1
            if time.monotonic() > deadline:
                raise AssertionError("Fixture installer did not exit after rejecting signature")
            time.sleep(0.25)
        feed(signature)
        command([alias, "update"])
        assert plistlib.loads((old / "Contents/Info.plist").read_bytes())["CFBundleVersion"] == "2"
        print("PASS: signed update replaces the closed app from a relocated bundle")
        command([alias, "update", "--check"], expected=4)

        running = temp / "Running App" / "TorroMail.app"
        shutil.copytree(template, running, symlinks=True)
        process = subprocess.Popen([str(running / "Contents/MacOS/TorroMailApp")],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 15
        while not pid_file.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise AssertionError("Windowless fixture app did not start")
            time.sleep(0.1)
        first_pid = int(pid_file.read_text())
        assert first_pid == process.pid
        command([running / "Contents/MacOS/torromail", "update"])
        process.wait(timeout=15)
        deadline = time.monotonic() + 15
        while int(pid_file.read_text()) == first_pid:
            if time.monotonic() > deadline:
                raise AssertionError("Updated windowless fixture app did not relaunch")
            time.sleep(0.1)
        new_pid = int(pid_file.read_text())
        assert str(running) in command(["ps", "-p", new_pid, "-o", "command="])
        os.kill(new_pid, signal.SIGTERM)
        assert plistlib.loads((running / "Contents/Info.plist").read_bytes())["CFBundleVersion"] == "2"
        print("PASS: a running app without a window is terminated, updated and relaunched")

        disabled = temp / "Disabled.app"
        shutil.copytree(template, disabled, symlinks=True)
        plist(disabled / "Contents/Info.plist", {"TorroMailDisableUpdates": True})
        sign(disabled)
        command([alias, "update", "--app", disabled], expected=1)
        wrong = temp / "Wrong.app"
        shutil.copytree(template, wrong, symlinks=True)
        plist(wrong / "Contents/Info.plist", {"CFBundleIdentifier": "com.example.other"})
        sign(wrong)
        command([alias, "update", "--app", wrong], expected=1)
        for name, changes in [("Bad Key.app", {"SUPublicEDKey": "invalid"}),
                              ("Bad Setting.app", {"TorroMailDisableUpdates": {"invalid": True}})]:
            broken = temp / name
            shutil.copytree(template, broken, symlinks=True)
            plist(broken / "Contents/Info.plist", changes)
            sign(broken)
            command([alias, "update", "--app", broken], expected=1)
        command([alias, "update", "--app", temp / "Missing.app"], expected=1)
        print("PASS: development, unrelated, malformed and missing apps are refused")
    finally:
        # Only our fixture application processes may be stopped, also on failure.
        processes = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
        for line in processes.splitlines():
            if str(temp) in line and "/Contents/MacOS/TorroMailApp" in line:
                try:
                    os.kill(int(line.strip().split(None, 1)[0]), signal.SIGTERM)
                except ProcessLookupError:
                    pass
        server.shutdown()
        # Only the unique fixture defaults domain is cleared.
        subprocess.run(["defaults", "delete", defaults_domain], capture_output=True)
print("CLI update integration tests passed")
