"""Two full-surface DP-2 screenshots; no in-surface warp sequence or tight crops."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

TARGET = Path('target/hyprswell')
ROOT = Path(__file__).resolve().parents[2]


def hypr(*args):
    return subprocess.run(['hyprctl', *args], capture_output=True, text=True,
                          check=True, timeout=2).stdout


def layers():
    return [(screen, layer) for screen, entry in json.loads(hypr('layers', '-j')).items()
            for rows in entry['levels'].values() for layer in rows
            if layer.get('namespace') == 'hyprswell']


def monitor():
    return next(m for m in json.loads(hypr('monitors', '-j')) if m['name'] == 'DP-2')


def move(x, y):
    assert hypr('dispatch', f'hl.dsp.cursor.move({{ x = {int(x)}, y = {int(y)} }})').strip() == 'ok'


def owned(pid):
    rows = [(screen, row) for screen, row in layers() if row.get('pid') == pid]
    assert len(rows) == 1 and rows[0][0] == 'DP-2', 'expected one test-owned DP-2 layer'
    return rows[0][1]


def capture(pid, path):
    row = owned(pid)
    geometry = f'{row["x"]},{row["y"]} {row["w"]}x{row["h"]}'
    subprocess.run(['grim', '-s', str(monitor()['scale']), '-g', geometry, str(path)],
                   check=True, capture_output=True, timeout=3)
    assert path.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'), 'capture is not a PNG'
    return row


def main():
    baseline = {(screen, row['pid'], row['address']) for screen, row in layers()}
    assert not any(screen == 'DP-2' for screen, _, _ in baseline), 'DP-2 already has a dock; do not stop it'
    before = monitor()['reserved']
    original_cursor = json.loads(hypr('cursorpos', '-j'))
    images = Path(tempfile.mkdtemp(prefix='reflections-images-', dir=TARGET))
    with tempfile.TemporaryDirectory(prefix='reflections-home-', dir=TARGET) as directory:
        home = Path(directory)
        config = home / 'config'
        (config / 'hyprswell').mkdir(parents=True)
        (config / 'nwg-dock-hyprland').mkdir()
        (home / 'cache').mkdir()
        (config / 'nwg-dock-hyprland/hyprswell.conf').write_text('ICON_SIZE=44\nREFLECTIONS=1\n')
        shutil.copyfile(ROOT / 'dock/style.example.css', config / 'hyprswell/style.css')
        pins = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache')) / 'nwg-dock-pinned'
        if pins.exists():
            shutil.copyfile(pins, home / 'cache/nwg-dock-pinned')
        env = os.environ.copy()
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(config), XDG_CACHE_HOME=str(home / 'cache'),
                   XDG_STATE_HOME=str(home / 'state'),
                   XDG_DATA_HOME=os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        child = None
        try:
            mon = monitor()
            move(mon['x'] + 2, mon['y'] + 80)
            with (home / 'stderr').open('w+') as errors:
                child = subprocess.Popen([str(TARGET / 'debug/hyprswell'), '--output', 'DP-2'],
                                         env=env, stdout=errors, stderr=errors)
                try:
                    deadline = time.monotonic() + 3
                    while not any(row.get('pid') == child.pid for _, row in layers()):
                        assert child.poll() is None, 'test dock exited before mapping'
                        assert time.monotonic() < deadline, 'no test layer within three seconds'
                        time.sleep(.05)
                    time.sleep(.6)
                    row = capture(child.pid, images / 'rest.png')
                    reserved = monitor()['reserved']
                    # One entry from outside, into the visible bar (not its transparent tail).
                    # Its centre is inside the input region even when crowded icons shrink.
                    move(row['x'] + row['w'] / 2,
                         row['y'] + row['h'] - 8 - (reserved[3] - 8) / 2)
                    time.sleep(.65)
                    assert child.poll() is None, 'test dock exited during hover'
                    grown = capture(child.pid, images / 'swollen.png')
                    assert monitor()['reserved'] == reserved, 'reservation changed during swell'
                    assert (grown['w'], grown['h']) == (row['w'], row['h']), 'surface resized during swell'
                    errors.flush()
                    errors.seek(0)
                    diagnostics = errors.read()
                    assert not any(word in diagnostics for word in ('CRITICAL', 'WARNING', 'panicked', 'Theme parser error')), diagnostics[:2500]
                    print(f'DP-2 surface {row["w"]}x{row["h"]}; reservation {reserved[3]} px stayed fixed')
                    print('Rest:', images / 'rest.png')
                    print('Swollen:', images / 'swollen.png')
                except Exception:
                    errors.flush()
                    errors.seek(0)
                    print(errors.read()[-2500:])
                    raise
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=2)
            move(original_cursor['x'], original_cursor['y'])
        assert child is not None
        deadline = time.monotonic() + 2
        while any(row.get('pid') == child.pid for _, row in layers()):
            assert time.monotonic() < deadline, 'test layer remained after PID cleanup'
            time.sleep(.05)
        assert monitor()['reserved'] == before, 'original reservation not restored'
        assert baseline == {(screen, row['pid'], row['address']) for screen, row in layers()}, 'live dock identities changed'
        print(f'Test PID {child.pid} stopped and reaped; live docks unchanged')


if __name__ == '__main__':
    main()
