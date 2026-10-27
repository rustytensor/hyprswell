"""Gate A's one live input: a new dock on DP-2, with PID-owned cleanup."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

TARGET = Path('target/hyprswell')


def layers():
    result = subprocess.run(['hyprctl', 'layers', '-j'], capture_output=True,
                            text=True, check=True, timeout=0.5)
    data = json.loads(result.stdout)
    return [(output, level, layer) for output, entry in data.items()
            for level, entries in entry['levels'].items() for layer in entries
            if layer.get('namespace') == 'hyprswell']


def main():
    before = layers()
    assert not any(output == 'DP-2' for output, _, _ in before), 'DP-2 already has a test dock'
    baseline = {entry['address'] for _, _, entry in before}
    with tempfile.TemporaryDirectory(prefix='surface-', dir=TARGET) as temp:
        home = Path(temp)
        env = os.environ.copy()
        env.update(HOME=temp, XDG_CONFIG_HOME=str(home / 'config'),
                   XDG_CACHE_HOME=str(home / 'cache'), XDG_STATE_HOME=str(home / 'state'))
        # Real desktop entries/icons remain readable; user pins and settings are isolated.
        for name in ('config', 'cache', 'state'):
            (home / name).mkdir()
        with (home / 'stderr').open('w+') as output:
            child = subprocess.Popen([str(TARGET / 'debug/hyprswell'), '--output', 'DP-2', '--no-swell'],
                                     env=env, stdout=output, stderr=output)
            started = time.monotonic()
            found = None
            try:
                deadline = started + 3
                while time.monotonic() < deadline:
                    if child.poll() is not None:
                        output.seek(0)
                        raise AssertionError(f'test dock exited {child.returncode}: {output.read(4000)}')
                    new = [(screen, level, row) for screen, level, row in layers()
                           if row['address'] not in baseline]
                    assert all(screen == 'DP-2' for screen, _, _ in new), 'dock appeared on another output'
                    matches = [(level, row) for _, level, row in new if row.get('pid') == child.pid]
                    if matches:
                        assert len(matches) == 1, 'duplicate dock surfaces'
                        level, found = matches[0]
                        assert level == '2', f'expected TOP layer 2, got {level}'
                        assert found['w'] > 0 and found['h'] > 0
                        assert time.monotonic() <= deadline, 'surface arrived after 3 seconds'
                        break
                    time.sleep(0.05)
                assert found is not None, 'no test-owned dock surface within 3 seconds'
                print(f"DP-2 TOP surface {found['w']}x{found['h']} in {time.monotonic()-started:.3f}s")
                output.flush()
                output.seek(0)
                diagnostics = output.read(4000)
                assert not any(word in diagnostics for word in ('CRITICAL', 'panicked', 'Theme parser error')), diagnostics
            finally:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=2)
            deadline = time.monotonic() + 2
            while any(row.get('pid') == child.pid for _, _, row in layers()):
                assert time.monotonic() < deadline, 'test surface remained after PID cleanup'
                time.sleep(0.05)
            print('Test PID stopped; its layer is gone')


if __name__ == '__main__':
    main()
