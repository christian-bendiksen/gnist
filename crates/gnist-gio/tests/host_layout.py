#!/usr/bin/env python3
"""End-to-end layout tests: real GTK 3 and GTK 4 hosts load gnist-gio.

Needs a C compiler, pkg-config with gtk4 and gtk+-3.0, Xvfb (xvfb-run) and
dbus-run-session. Run from anywhere:

    python3 crates/gnist-gio/tests/host_layout.py
"""
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent
WORKSPACE = CRATE.parent.parent
TIMEOUT = 10.0


def layout_doc(layout, **overrides):
    doc = {'schema': 'herda.gtk-layout', 'version': 1, 'layout': layout}
    doc.update(overrides)
    return json.dumps(doc)


def replace_atomically(path, text):
    """Write the way Malm does: a sibling file renamed over the target."""
    path.parent.mkdir(parents=True, exist_ok=True)
    staging = path.with_name('.gtk-layout.json.staging')
    staging.write_text(text)
    os.replace(staging, path)


class Host:
    def __init__(self, binary, env):
        self.process = subprocess.Popen(
            [binary], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, bufsize=1)
        self.lines = queue.Queue()
        self.stderr = []
        threading.Thread(target=self._pump, args=(self.process.stdout, self.lines.put), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.process.stderr, self.stderr.append), daemon=True).start()
        self.expect('WINDOW 1')
        self.expect('READY')

    @staticmethod
    def _pump(stream, sink):
        for line in stream:
            sink(line.rstrip('\n'))

    def expect(self, prefix, timeout=TIMEOUT):
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f'no {prefix!r} line; stderr:\n' + '\n'.join(self.stderr))
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                continue
            if line.startswith(prefix):
                return line

    def quiet(self, seconds=0.6):
        """Nothing is printed for a while: no spurious notifications."""
        time.sleep(seconds)
        leftovers = []
        while not self.lines.empty():
            leftovers.append(self.lines.get())
        assert not leftovers, leftovers

    def window(self):
        self.process.stdin.write('window\n')
        return self.expect('WINDOW').split()[2]

    def wait_log(self, needle, timeout=TIMEOUT):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if any(needle in line for line in self.stderr):
                return
            time.sleep(0.05)
        raise AssertionError(f'no {needle!r} in stderr:\n' + '\n'.join(self.stderr))

    def close(self):
        self.process.stdin.write('quit\n')
        self.process.stdin.close()
        assert self.process.wait(TIMEOUT) == 0, self.stderr
        problems = [line for line in self.stderr if 'CRITICAL' in line or 'WARNING' in line]
        assert not problems, problems


def build(scratch, gtk):
    package = {3: 'gtk+-3.0', 4: 'gtk4'}[gtk]
    flags = subprocess.run(['pkg-config', '--cflags', '--libs', package],
                           check=True, capture_output=True, text=True).stdout.split()
    binary = scratch / f'layout-host-gtk{gtk}'
    subprocess.run(['cc', '-Wall', '-Werror', '-o', str(binary), str(HERE / 'host/layout-host.c'), *flags],
                   check=True)
    return binary


def scenario(binary, modules, scratch, *, patched, early):
    home = Path(tempfile.mkdtemp(dir=scratch))
    config = home / 'config'
    path = config / 'herda/gtk-layout.json'
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(config),
               GIO_EXTRA_MODULES=str(modules), GTK_GNIST_DEBUG='1', GNIST_TRANSITION_MS='0',
               LAYOUT_HOST_PATCHED='1' if patched else '0',
               LAYOUT_HOST_EARLY_MODULES='1' if early else '0')
    env.pop('GTK_DISABLE_GNIST_STYLE', None)
    name = f'{binary.name} patched={patched} early={early}'

    if not patched:
        replace_atomically(path, layout_doc('herda-desktop'))
        host = Host(binary, env)
        host.wait_log('toolkit has no gtk-herda-layout')
        replace_atomically(path, layout_doc('native'))
        host.wait_log('not published (file)')
        assert host.window() == 'unsupported'
        host.close()
        print(f'ok {name}: unpatched toolkit left alone')
        return

    # The file exists before the application starts: its first window may be
    # built before the module attaches, and must be re-laid out once it does.
    replace_atomically(path, layout_doc('herda-desktop'))
    host = Host(binary, env)
    assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'
    assert host.window() == 'herda-desktop', 'a later window reads the published layout'
    host.quiet()

    # A profile switch to Seam, as an atomic rename over the file.
    replace_atomically(path, layout_doc('native'))
    assert host.expect('NOTIFY') == 'NOTIFY native'
    assert host.window() == 'native'

    # Rewriting the same layout -- a theme-only deployment -- changes nothing.
    replace_atomically(path, layout_doc('native'))
    host.wait_log('layout=native unchanged (file)')
    host.quiet()

    # Back to Desktop by an in-place write rather than a rename.
    path.write_text(layout_doc('herda-desktop'))
    assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'

    # Malformed, future and unknown documents all mean native.
    for text in ['{"schema":', layout_doc('herda-desktop', version=2), layout_doc('windows'), '[]']:
        replace_atomically(path, text)
        assert host.expect('NOTIFY') == 'NOTIFY native', text
        replace_atomically(path, layout_doc('herda-desktop'))
        assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'

    # Removing the file is the native layout; so is removing its directory.
    path.unlink()
    assert host.expect('NOTIFY') == 'NOTIFY native'
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'
    shutil.rmtree(path.parent)
    assert host.expect('NOTIFY') == 'NOTIFY native'
    path.parent.mkdir()
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY', timeout=TIMEOUT) == 'NOTIFY herda-desktop'

    # The D-Bus nudge and a colour change both re-read the file and converge.
    def signal(*args):
        subprocess.run(['dbus-send', '--session', '--type=signal', *args], check=True, env=env)

    signal('/org/gnist/Layout1', 'org.gnist.Layout1.Changed', 'uint64:7', 'string:herda-desktop')
    host.wait_log('layout=herda-desktop unchanged (signal)')
    css = config / 'gnist/themes/current/gtk.css'
    css.parent.mkdir(parents=True, exist_ok=True)
    css.write_text('window { }\n')
    signal('/org/gnist/Appearance1', 'org.gnist.Appearance1.Changed', 'uint64:3', f'string:{css}', 'string:dark')
    host.wait_log('layout=herda-desktop unchanged (appearance)')
    host.wait_log('reloaded GTK')
    host.quiet()
    assert host.window() == 'herda-desktop', 'a colour change leaves the layout alone'

    host.close()
    print(f'ok {name}')


def main():
    for tool in ['cc', 'pkg-config', 'xvfb-run', 'dbus-run-session', 'dbus-send', 'cargo']:
        if not shutil.which(tool):
            sys.exit(f'missing {tool}')
    if os.environ.get('LAYOUT_HOST_INNER') != '1':
        subprocess.run(['cargo', 'build', '-p', 'gnist-gio'], cwd=WORKSPACE, check=True)
        env = dict(os.environ, LAYOUT_HOST_INNER='1', GDK_BACKEND='x11', NO_AT_BRIDGE='1', GTK_A11Y='none')
        for key in ['WAYLAND_DISPLAY', 'DISPLAY', 'DBUS_SESSION_BUS_ADDRESS']:
            env.pop(key, None)
        sys.exit(subprocess.run(['xvfb-run', '-a', 'dbus-run-session', '--', sys.executable, __file__],
                                env=env).returncode)

    target = Path(os.environ.get('CARGO_TARGET_DIR', WORKSPACE / 'target')) / 'debug/libgnist_gio.so'
    with tempfile.TemporaryDirectory(prefix='gnist-layout-') as temporary:
        scratch = Path(temporary)
        modules = scratch / 'modules'
        modules.mkdir()
        shutil.copy2(target, modules / 'libgnist_gio.so')
        for gtk in [4, 3]:
            binary = build(scratch, gtk)
            for patched in [True, False]:
                for early in [False, True]:
                    scenario(binary, modules, scratch, patched=patched, early=early)


if __name__ == '__main__':
    main()
