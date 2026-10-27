"""Bounded DP-2 recording. Human visual acceptance is still required."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

TARGET = Path('target/hyprswell')
BINARY = TARGET / 'debug/hyprswell'


def hypr(*args):
    return subprocess.run(['hyprctl', *args], capture_output=True, text=True,
                          check=True, timeout=2).stdout


def move(x, y):
    reply = hypr('dispatch', f'hl.dsp.cursor.move({{ x = {int(x)}, y = {int(y)} }})')
    assert reply.strip() == 'ok', reply


def layers():
    data = json.loads(hypr('layers', '-j'))
    return [(screen, row) for screen, entry in data.items()
            for rows in entry['levels'].values() for row in rows
            if row.get('namespace') == 'hyprswell']


def monitor():
    return next(m for m in json.loads(hypr('monitors', '-j')) if m['name'] == 'DP-2')


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=2)


def record(env, home, plain):
    label = 'no-swell' if plain else 'swell'
    args = [str(BINARY), '--output', 'DP-2'] + (['--no-swell'] if plain else [])
    with (home / f'{label}.log').open('w+') as log:
        process = subprocess.Popen(args, env=env, stdout=log, stderr=log)
        encoder = None
        try:
            deadline = time.monotonic() + 3
            row = None
            while time.monotonic() < deadline:
                assert process.poll() is None, f'{label}: test dock exited'
                owned = [(screen, layer) for screen, layer in layers() if layer.get('pid') == process.pid]
                if owned:
                    assert len(owned) == 1 and owned[0][0] == 'DP-2'
                    row = owned[0][1]
                    break
                time.sleep(0.05)
            assert row is not None, 'dock did not appear on DP-2 within 3 seconds'
            # Let the initial client snapshot resize the surface before fixing the crop.
            time.sleep(0.6)
            row = next(layer for _, layer in layers() if layer.get('pid') == process.pid)
            x, y, width, height = (row[k] for k in ('x', 'y', 'w', 'h'))
            geometry = f'{x},{y} {width}x{height}'
            reserved = monitor()['reserved']
            assert reserved[3] > 0, 'bar must reserve space above the bottom edge'
            encoder = subprocess.Popen(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y',
                '-f', 'image2pipe', '-framerate', '20', '-vcodec', 'png', '-i', '-',
                '-vf', 'pad=ceil(iw/2)*2:ceil(ih/2)*2', '-c:v', 'libx264', '-threads', '2',
                '-preset', 'fast', '-pix_fmt', 'yuv420p', str(TARGET / f'{label}.mp4')],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=log)
            started = time.monotonic()
            for frame in range(100):
                t = frame / 20
                if 0.5 <= t < 2.5:
                    fraction = (t - 0.5) / 2
                    move(x + width * (0.25 + 0.5 * fraction), y + height - 35)
                elif 2.5 <= t < 3.5:
                    move(x + width / 2, y + height - 35)
                else:
                    move(x + 2, y - 20)
                assert process.poll() is None, f'{label}: test dock exited while recording'
                image = subprocess.run(['grim', '-s', '1', '-g', geometry, '-'],
                                       capture_output=True, check=True, timeout=2).stdout
                if frame in (5, 65):
                    (TARGET / f'{label}-{"rest" if frame == 5 else "hover"}.png').write_bytes(image)
                encoder.stdin.write(image)
                if frame in (5, 40, 65, 90):
                    # Motion must never change the tiled-window reservation.
                    assert monitor()['reserved'] == reserved, 'exclusive zone changed during animation'
                time.sleep(max(0, started + (frame + 1) / 20 - time.monotonic()))
            encoder.stdin.close()
            assert encoder.wait(timeout=10) == 0, 'video encoder failed'
            log.flush()
            log.seek(0)
            diagnostics = log.read()
            assert not any(word in diagnostics for word in ('CRITICAL', 'panicked', 'Theme parser error')), diagnostics[:3000]
            print(f'{label}: DP-2 {width}x{height}, reservation {reserved[3]}, 100 frames; PID {process.pid}')
        finally:
            if encoder is not None:
                stop(encoder)
            stop(process)
            log.flush()
            log.seek(0)
            errors = log.read()
            if errors:
                print(errors[:3000])
        deadline = time.monotonic() + 2
        while any(layer.get('pid') == process.pid for _, layer in layers()):
            assert time.monotonic() < deadline, 'test layer remained after cleanup'
            time.sleep(0.05)


def main():
    assert not any(screen == 'DP-2' for screen, _ in layers()), 'DP-2 already has a test dock'
    old = json.loads(hypr('cursorpos', '-j'))
    before = monitor()['reserved']
    with tempfile.TemporaryDirectory(prefix='visual-', dir=TARGET) as temp:
        home = Path(temp)
        env = os.environ.copy()
        env.update(HOME=temp, XDG_CONFIG_HOME=str(home / 'config'),
                   XDG_CACHE_HOME=str(home / 'cache'), XDG_STATE_HOME=str(home / 'state'),
                   XDG_DATA_HOME=os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        (home / 'cache').mkdir()
        style = home / 'config/hyprswell'
        style.mkdir(parents=True)
        shutil.copyfile(Path(__file__).resolve().parents[1] / 'style.example.css', style / 'style.css')
        pins = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache')) / 'nwg-dock-pinned'
        if pins.exists():
            shutil.copyfile(pins, home / 'cache/nwg-dock-pinned')
        try:
            mon = monitor()
            move(mon['x'] + 2, mon['y'] + 60)
            record(env, home, False)
            record(env, home, True)
        finally:
            move(old['x'], old['y'])
    assert monitor()['reserved'] == before, 'reservation was not restored after stopping test docks'
    print('Recording checks PASS; both test PIDs stopped. Visual acceptance pending.')


if __name__ == '__main__':
    main()
