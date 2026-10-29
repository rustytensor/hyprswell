"""Four rest captures, one owned DP-2 dock at a time. No live preferences are changed."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from reflections_capture import TARGET, ROOT, hypr, layers, monitor, move, wait_settled, capture


def stop(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=2)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=2)


def main():
    baseline = {(screen, row['pid'], row['address']) for screen, row in layers()}
    assert not any(screen == 'DP-2' for screen, _, _ in baseline), 'DP-2 already has a dock; leave it alone'
    before = monitor()['reserved']
    original = json.loads(hypr('cursorpos', '-j'))
    images = Path(tempfile.mkdtemp(prefix='launcher-images-', dir=TARGET))
    pin_file = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache')) / 'nwg-dock-pinned'
    pins = pin_file.read_text() if pin_file.exists() else ''
    try:
        for mode in ('left', 'right', 'center', 'off'):
            with tempfile.TemporaryDirectory(prefix='launcher-home-', dir=TARGET) as directory:
                home = Path(directory)
                config = home / 'config'
                (config / 'nwg-dock-hyprland').mkdir(parents=True)
                (config / 'hyprswell').mkdir()
                (home / 'cache').mkdir()
                (config / 'nwg-dock-hyprland/hyprswell.conf').write_text(f'ICON_SIZE=44\nREFLECTIONS=1\nLAUNCHER={mode}\n')
                (home / 'cache/nwg-dock-pinned').write_text(pins)
                shutil.copyfile(ROOT / 'dock/style.example.css', config / 'hyprswell/style.css')
                env = os.environ.copy()
                env.update(HOME=str(home), XDG_CONFIG_HOME=str(config), XDG_CACHE_HOME=str(home / 'cache'),
                           XDG_STATE_HOME=str(home / 'state'),
                           XDG_DATA_HOME=os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
                mon = monitor()
                move(mon['x'] + 2, mon['y'] + 80)
                with (home / 'stderr').open('w+') as errors:
                    child = subprocess.Popen([str(TARGET / 'debug/hyprswell'), '--output', 'DP-2'],
                                             env=env, stdout=errors, stderr=errors)
                    print(f'{mode} test PID: {child.pid}', flush=True)
                    try:
                        deadline = time.monotonic() + 3
                        while not any(row.get('pid') == child.pid for _, row in layers()):
                            assert child.poll() is None, 'test dock exited before mapping'
                            assert time.monotonic() < deadline, 'no owned layer in three seconds'
                            time.sleep(.05)
                        settled, reserved = wait_settled(child)
                        path = images / f'{mode}.png'
                        row = capture(child.pid, path)
                        print(f'{mode}: {row["w"]}x{row["h"]}, reservation {reserved}; {path}', flush=True)
                        assert all(row[k] == settled[k] for k in ('x', 'y', 'w', 'h')), 'layout changed during capture'
                        errors.flush(); errors.seek(0)
                        text = errors.read()
                        assert not any(word in text for word in ('CRITICAL', 'WARNING', 'panicked', 'Theme parser error')), text[-2500:]
                    except Exception:
                        errors.flush(); errors.seek(0)
                        print(errors.read()[-2500:])
                        raise
                    finally:
                        stop(child)
                        deadline = time.monotonic() + 2
                        while any(row.get('pid') == child.pid for _, row in layers()):
                            assert time.monotonic() < deadline, 'owned layer remained after cleanup'
                            time.sleep(.05)
                        print(f'{mode} PID {child.pid} stopped and reaped', flush=True)
                assert monitor()['reserved'] == before, 'original reservation not restored'
                assert baseline == {(screen, row['pid'], row['address']) for screen, row in layers()}, 'live dock identities changed'
    finally:
        move(original['x'], original['y'])


if __name__ == '__main__':
    main()
