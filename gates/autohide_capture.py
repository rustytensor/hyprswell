"""Auto-hide capture: one owned DP-2 dock with AUTOHIDE=1, a private control socket and --no-swell.
No live preferences, production dock or production control endpoint are touched."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time

from reflections_capture import TARGET, ROOT, hypr, layers, monitor, move, owned, wait_settled, capture
from launcher_capture import stop

DOCK = TARGET / 'debug/hyprswell'


def production_socket():
    runtime, instance = os.environ.get('XDG_RUNTIME_DIR', ''), os.environ.get('HYPRLAND_INSTANCE_SIGNATURE', '')
    return Path(runtime) / 'hypr' / instance / 'hyprswell.sock'


def main():
    baseline = {(screen, row['pid'], row['address']) for screen, row in layers()}
    assert not any(screen == 'DP-2' for screen, _, _ in baseline), 'DP-2 already has a dock; leave it alone'
    before = monitor()['reserved']
    original = json.loads(hypr('cursorpos', '-j'))
    images = Path(tempfile.mkdtemp(prefix='autohide-images-', dir=TARGET))
    child = None
    with tempfile.TemporaryDirectory(prefix='autohide-home-', dir=TARGET) as directory:
        home = Path(directory)
        config = home / 'config'
        (config / 'nwg-dock-hyprland').mkdir(parents=True)
        (config / 'hyprswell').mkdir()
        (home / 'cache').mkdir()
        (config / 'nwg-dock-hyprland/hyprswell.conf').write_text('ICON_SIZE=44\nREFLECTIONS=1\nAUTOHIDE=1\n')
        shutil.copyfile(ROOT / 'dock/style.example.css', config / 'hyprswell/style.css')
        pins = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache')) / 'nwg-dock-pinned'
        if pins.exists():
            shutil.copyfile(pins, home / 'cache/nwg-dock-pinned')
        socket = home / 'control.sock'
        assert socket.resolve() != production_socket().resolve(), 'private control socket required'
        env = os.environ.copy()
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(config), XDG_CACHE_HOME=str(home / 'cache'),
                   XDG_STATE_HOME=str(home / 'state'),
                   XDG_DATA_HOME=os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        try:
            mon = monitor()
            move(mon['x'] + 2, mon['y'] + 80)
            with (home / 'stderr').open('w+') as errors:
                child = subprocess.Popen([str(DOCK), '--output', 'DP-2', '--no-swell', '--control-socket', str(socket)],
                                         env=env, stdout=errors, stderr=errors)
                print(f'Test PID: {child.pid}', flush=True)
                try:
                    deadline = time.monotonic() + 3
                    while not any(row.get('pid') == child.pid for _, row in layers()):
                        assert child.poll() is None, 'test dock exited before mapping'
                        assert time.monotonic() < deadline, 'no owned layer in three seconds'
                        time.sleep(.05)
                    settled, reserved = wait_settled(child)
                    print(f'Hidden reservation: {reserved} (bottom {reserved[3]}, expected 6)', flush=True)
                    assert reserved[3] == 6, f'hidden reservation {reserved[3]}, expected 6'
                    row = capture(child.pid, images / 'hidden.png')
                    assert all(row[k] == settled[k] for k in ('x', 'y', 'w', 'h')), 'layout changed before hidden capture'
                    # Reservation sampled throughout the reveal: it must not move during the animation.
                    samples, done = [], threading.Event()

                    def sample():
                        while not done.is_set():
                            samples.append(tuple(monitor()['reserved']))
                            time.sleep(.02)
                    sampler = threading.Thread(target=sample)
                    sampler.start()
                    started = time.monotonic()
                    try:
                        shown = subprocess.run([str(DOCK), '--show', '--control-socket', str(socket)], env=env,
                                               capture_output=True, text=True, timeout=8)
                    finally:
                        done.set()
                        sampler.join()
                    print(f'--show returned {shown.returncode} after {time.monotonic() - started:.3f}s '
                          f'{shown.stderr.strip()!r}; {len(samples)} reservation samples', flush=True)
                    assert shown.returncode == 0, 'show request was not acknowledged with a shown frame'
                    assert samples and set(samples) == {tuple(reserved)}, f'reservation moved: {sorted(set(samples))}'
                    moved = owned(child.pid)
                    assert all(moved[k] == settled[k] for k in ('x', 'y', 'w', 'h')), 'native surface moved or resized'
                    # Enter the visible bar once, in its reflection lane below the icons, to hold it open
                    # past the 700 ms hide delay without hovering an icon.
                    move(moved['x'] + moved['w'] / 2, moved['y'] + moved['h'] - 14)
                    time.sleep(1.0)
                    assert child.poll() is None, 'test dock exited while shown'
                    capture(child.pid, images / 'shown.png')
                    after = monitor()['reserved']
                    print(f'Shown reservation: {after} (bottom {after[3]}, expected 6)', flush=True)
                    assert after == reserved, 'reservation changed between hidden and shown'
                    errors.flush()
                    errors.seek(0)
                    diagnostics = errors.read()
                    assert not any(word in diagnostics for word in ('CRITICAL', 'WARNING', 'panicked', 'Theme parser error')), diagnostics[-2500:]
                    print(f'Surface {settled["w"]}x{settled["h"]} fixed in both poses')
                except Exception:
                    errors.flush()
                    errors.seek(0)
                    print(errors.read()[-2500:])
                    raise
        finally:
            if child is not None:
                stop(child)
            move(original['x'], original['y'])
        assert child is not None
        deadline = time.monotonic() + 2
        while any(row.get('pid') == child.pid for _, row in layers()):
            assert time.monotonic() < deadline, 'owned layer remained after cleanup'
            time.sleep(.05)
        assert not socket.exists(), 'private control socket left behind'
    print(f'Test PID {child.pid} stopped and reaped', flush=True)
    restored = monitor()['reserved']
    print(f'DP-2 reservation after cleanup: {restored} (before {before})', flush=True)
    assert restored == before, 'original reservation not restored'
    assert baseline == {(screen, row['pid'], row['address']) for screen, row in layers()}, 'live dock identities changed'
    print('Hidden:', images / 'hidden.png')
    print('Shown:', images / 'shown.png')


if __name__ == '__main__':
    main()
