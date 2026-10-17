#!/usr/bin/env python3
"""End-to-end layout tests: real GTK 3 and GTK 4 hosts load gnist-gio.

Needs a C compiler, pkg-config with gtk4 and gtk+-3.0, Xvfb (xvfb-run) and
dbus-run-session. Run from anywhere:

    python3 crates/gnist-gio/tests/host_layout.py

By default the toolkit is upstream GTK and the host stands in for the
patched property. Against Herda's patched GTK, put its libraries first on
LD_LIBRARY_PATH and set

    LAYOUT_HOST_TOOLKIT=real   the toolkit's own property and startup read;
                               the stand-in and unpatched cases are skipped
    LAYOUT_HOST_GTK=4          which GTKs to test (default 4,3)
    LAYOUT_HOST_EXPECT_LIB=/usr/lib/herda/gtk-layout
                               fail unless libgtk is loaded from there
    GNIST_GIO_MODULE=/path/libgnist_gio.so
                               a module built elsewhere (default: cargo build)

Each host prints the libgtk, GLib and gnist-gio it actually mapped.
"""
import hashlib
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


def layout_doc(layout, schema='org.herda.GtkLayout', version='1'):
    return f'[GtkLayout]\nSchema={schema}\nVersion={version}\nLayout={layout}\n'


def replace_atomically(path, text):
    """Write the way Malm does: a sibling file renamed over the target."""
    path.parent.mkdir(parents=True, exist_ok=True)
    staging = path.with_name('.gtk-layout.ini.staging')
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
        self.libraries = []
        while (line := self.expect('', timeout=TIMEOUT)) != 'LIB-END' and not line.startswith('WINDOW 1'):
            if line.startswith('LIB '):
                self.libraries.append(line[4:])
        if not line.startswith('WINDOW 1'):
            line = self.expect('WINDOW 1')
        self.first_window = line.split()[2]
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

    def opacity(self):
        self.process.stdin.write('opacity\n')
        return [float(value) for value in self.expect('OPACITY').split()[1:]]

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


REAL = os.environ.get('LAYOUT_HOST_TOOLKIT') == 'real'


def stand_in(patched):
    """LAYOUT_HOST_PATCHED for a case that wants the property."""
    return ('real' if REAL else '1') if patched else '0'


def check_libraries(host, name, reported=set()):
    if name in reported:
        return
    reported.add(name)
    for library in host.libraries:
        print(f'   {name} loaded {library}')
    expected = os.environ.get('LAYOUT_HOST_EXPECT_LIB')
    if expected:
        gtk = [library for library in host.libraries if '/libgtk' in library]
        assert gtk and all(library.startswith(expected) for library in gtk), gtk


def scenario(binary, modules, scratch, *, patched, load, early_read):
    home = Path(tempfile.mkdtemp(dir=scratch))
    config = home / 'config'
    path = config / 'herda/gtk-layout.ini'
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(config), GTK_GNIST_DEBUG='1',
               GNIST_TRANSITION_MS='0', LAYOUT_HOST_PATCHED=stand_in(patched),
               LAYOUT_HOST_MODULES=load, LAYOUT_HOST_EARLY_READ='1' if early_read else '0')
    if load != 'absent':
        env['GIO_EXTRA_MODULES'] = str(modules)
    env.pop('GTK_DISABLE_GNIST_STYLE', None)
    name = f'{binary.name} patched={patched} modules={load} early-read={early_read}'

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

    # The file exists before the application starts.
    replace_atomically(path, layout_doc('herda-desktop'))
    host = Host(binary, env)
    check_libraries(host, binary.name)
    if load == 'absent':
        # Only the toolkit's own read: right at first paint, no live changes.
        assert host.first_window == ('herda-desktop' if early_read else 'native')
        replace_atomically(path, layout_doc('native'))
        host.quiet()
        host.close()
        print(f'ok {name}')
        return
    if early_read:
        # The toolkit had it right for the first window; attaching agrees.
        assert host.first_window == 'herda-desktop'
        host.wait_log('layout=herda-desktop unchanged (attach)')
    else:
        # A first window built before attach is re-laid out once it attaches.
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
    for text in ['[GtkLayout', layout_doc('herda-desktop', version='2'), layout_doc('windows'),
                 layout_doc('herda-desktop', schema='org.herda.Other'), '']:
        replace_atomically(path, text)
        assert host.expect('NOTIFY') == 'NOTIFY native', text
        replace_atomically(path, layout_doc('herda-desktop'))
        assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'

    # A FIFO in its place is native, and neither freezes the application nor
    # sets the directory monitor reading in a loop.
    path.unlink()
    os.mkfifo(path)
    assert host.expect('NOTIFY') == 'NOTIFY native'
    time.sleep(1.0)
    events = sum('layout event' in line for line in host.stderr)
    time.sleep(1.0)
    assert sum('layout event' in line for line in host.stderr) == events, 'monitor loops on a FIFO'
    assert host.window() == 'native', 'the application still answers'
    path.unlink()
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'

    # Removing the file is native; so is removing its directory, and both
    # coming back -- directory first, then the file -- is followed live.
    path.unlink()
    assert host.expect('NOTIFY') == 'NOTIFY native'
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY') == 'NOTIFY herda-desktop'
    shutil.rmtree(config)
    assert host.expect('NOTIFY') == 'NOTIFY native'
    config.mkdir()
    host.quiet(0.3)
    path.parent.mkdir()
    host.quiet(0.3)
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY', timeout=1.5) == 'NOTIFY herda-desktop'

    # A colour change re-reads the file but leaves an unchanged layout alone.
    css = config / 'gnist/themes/current/gtk.css'
    css.parent.mkdir(parents=True, exist_ok=True)
    css.write_text('window { }\n')
    subprocess.run(['dbus-send', '--session', '--type=signal', '/org/gnist/Appearance1',
                    'org.gnist.Appearance1.Changed', 'uint64:3', f'string:{css}', 'string:dark'],
                   check=True, env=env)
    host.wait_log('layout=herda-desktop unchanged (appearance)')
    host.wait_log('reloaded GTK')
    host.quiet()
    assert host.window() == 'herda-desktop'

    host.close()
    print(f'ok {name}')


def missing_directory(binary, modules, scratch):
    """Start with no configuration directory at all, then create it."""
    home = Path(tempfile.mkdtemp(dir=scratch))
    config = home / 'config'
    path = config / 'herda/gtk-layout.ini'
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(config), GTK_GNIST_DEBUG='1',
               GIO_EXTRA_MODULES=str(modules), LAYOUT_HOST_PATCHED=stand_in(True), LAYOUT_HOST_MODULES='late',
               LAYOUT_HOST_EARLY_READ='1')
    host = Host(binary, env)
    assert host.first_window == 'native'
    host.wait_log('layout=native unchanged (attach)')
    config.mkdir()
    host.quiet(0.3)
    path.parent.mkdir()
    host.quiet(0.3)
    # Faster than GLib's own four-second poll for missing directories: the
    # monitor moved down to the new directory.
    replace_atomically(path, layout_doc('herda-desktop'))
    assert host.expect('NOTIFY', timeout=1.5) == 'NOTIFY herda-desktop'
    path.unlink()
    assert host.expect('NOTIFY', timeout=1.5) == 'NOTIFY native'
    host.close()
    print(f'ok {binary.name} missing configuration directory')


def crossfade(binary, modules, scratch):
    """A colour change fades the title bar with the content, then restores both."""
    home = Path(tempfile.mkdtemp(dir=scratch))
    config = home / 'config'
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(config), GTK_GNIST_DEBUG='1',
               GIO_EXTRA_MODULES=str(modules), GNIST_TRANSITION_MS='1200', LAYOUT_HOST_MODULES='late',
               LAYOUT_HOST_PATCHED=stand_in(REAL))
    css = config / 'gnist/themes/current/gtk.css'
    css.parent.mkdir(parents=True)
    css.write_text('window { }\n')
    host = Host(binary, env)
    host.wait_log('subscribed to D-Bus')
    before = host.opacity()
    assert len(before) >= 2 and all(value == 1.0 for value in before), before
    subprocess.run(['dbus-send', '--session', '--type=signal', '/org/gnist/Appearance1',
                    'org.gnist.Appearance1.Changed', 'uint64:9', f'string:{css}', 'string:dark'],
                   check=True, env=env)
    host.wait_log('fade started')
    time.sleep(0.3)
    during = host.opacity()
    assert len(during) == len(before) and all(value < 0.95 for value in during), during
    time.sleep(1.5)
    after = host.opacity()
    assert all(value == 1.0 for value in after), after

    # Reduced motion: the same change is instant.
    host.process.stdin.write('reduce-motion\n')
    subprocess.run(['dbus-send', '--session', '--type=signal', '/org/gnist/Appearance1',
                    'org.gnist.Appearance1.Changed', 'uint64:10', f'string:{css}', 'string:light'],
                   check=True, env=env)
    host.wait_log('theme notify complete: reduced-motion')
    assert all(value == 1.0 for value in host.opacity())
    host.close()
    print(f'ok {binary.name} crossfade: {len(before)} children faded together and restored; reduced motion instant')


def main():
    tools = ['cc', 'pkg-config', 'xvfb-run', 'dbus-run-session', 'dbus-send']
    for tool in tools + ([] if os.environ.get('GNIST_GIO_MODULE') else ['cargo']):
        if not shutil.which(tool):
            sys.exit(f'missing {tool}')
    if os.environ.get('LAYOUT_HOST_INNER') != '1':
        if not os.environ.get('GNIST_GIO_MODULE'):
            subprocess.run(['cargo', 'build', '-p', 'gnist-gio'], cwd=WORKSPACE, check=True)
        env = dict(os.environ, LAYOUT_HOST_INNER='1', GDK_BACKEND='x11', NO_AT_BRIDGE='1', GTK_A11Y='none')
        for key in ['WAYLAND_DISPLAY', 'DISPLAY', 'DBUS_SESSION_BUS_ADDRESS']:
            env.pop(key, None)
        sys.exit(subprocess.run(['xvfb-run', '-a', 'dbus-run-session', '--', sys.executable, __file__],
                                env=env).returncode)

    target = Path(os.environ.get('GNIST_GIO_MODULE') or
                  Path(os.environ.get('CARGO_TARGET_DIR', WORKSPACE / 'target')) / 'debug/libgnist_gio.so')
    print(f'module {target} sha256 {hashlib.sha256(target.read_bytes()).hexdigest()}')
    with tempfile.TemporaryDirectory(prefix='gnist-layout-') as temporary:
        scratch = Path(temporary)
        modules = scratch / 'modules'
        modules.mkdir()
        shutil.copy2(target, modules / 'libgnist_gio.so')
        for gtk in [int(major) for major in os.environ.get('LAYOUT_HOST_GTK', '4,3').split(',')]:
            binary = build(scratch, gtk)
            crossfade(binary, modules, scratch)
            missing_directory(binary, modules, scratch)
            # A patched toolkit always reads the file at startup itself.
            for load in ['late', 'early', 'thread', 'absent']:
                for early_read in [True] if REAL else [False, True]:
                    scenario(binary, modules, scratch, patched=True, load=load, early_read=early_read)
            if not REAL:
                for load in ['late', 'early']:
                    scenario(binary, modules, scratch, patched=False, load=load, early_read=False)


if __name__ == '__main__':
    main()
