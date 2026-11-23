#!/usr/bin/env python3
"""Runs the TeX tests: <root>/tests/tex/<package>/*.tex, each built with
uplatex and dviavi in a scratch directory, then checked against its `%!` lines:

    %! engine: E           uplatex (default) or platex
    %! pages: N            the DVI has N pages (and dviavi makes N frames)
    %! errors: N           the log has N errors (default 0)
    %! log: TEXT           the log has TEXT;  `%! log: TEXT = N`  N times
    %! special: TEXT = N   N specials containing TEXT
    %! compare: P, ...     frame P (1-based) looks like dvipdfmx's page P: at most
                           0.1% of its pixels differ by more than 48 (of 255)

usage: tests/tex/run.py [--root DIR] [NAME ...]   (names match files, like 05 or plain)

--root is the repository whose tests run (default: dviavi's own); TeX finds
its tex/ and dviavi's tex/.  The dviavi program is $DVIAVI, else this
repository's target/release/dviavi.
"""
import re, shutil, subprocess, sys, tempfile
from pathlib import Path

import os

HERE = Path(__file__).resolve().parents[2]   # the dviavi repository
DVIAVI = Path(os.environ.get('DVIAVI', HERE / 'target' / 'release' / 'dviavi'))
TIMEOUT = 120  # a test that runs longer has looped


def specials(dvi: bytes):
    """The xxx strings of a DVI file (good enough for counting)."""
    out, i = [], 0
    while True:
        i = dvi.find(b'\xef', i)  # xxx1
        if i < 0:
            return out
        n = dvi[i + 1]
        s = dvi[i + 2:i + 2 + n]
        if s.startswith((b'dviavi:', b'pdf:', b'color', b'papersize', b'background')):
            out.append(s.decode('utf-8', 'replace'))
            i += 2 + n
        else:
            i += 1


def frame_diff(work: Path, avi: Path, page: int) -> float:
    """Percentage of pixels of dviavi's frame that differ by more than 48 (of
    255) from dvipdfmx's page; dithering and antialiasing stay below."""
    from PIL import Image, ImageChops
    run(['dvipdfmx', '-q', '-o', 'ref.pdf', avi.with_suffix('.dvi').name], work)
    w, h = Image.open(png_of_frame(work, avi, page)).size
    run(['pdftoppm', '-png', '-r', '300', '-f', str(page), '-l', str(page), '-singlefile', 'ref.pdf', 'ref'], work)
    ref = Image.open(work / 'ref.png').convert('RGB').resize((w, h), Image.LANCZOS)
    got = Image.open(work / f'frame{page}.png').convert('RGB')
    diff = ImageChops.difference(ref, got).convert('L')
    return 100 * sum(1 for v in diff.getdata() if v > 48) / (w * h)


def png_of_frame(work: Path, avi: Path, page: int) -> Path:
    out = work / f'frame{page}.png'
    run(['ffmpeg', '-v', 'error', '-y', '-i', avi.name, '-vf', f'select=eq(n\\,{page - 1})', '-frames:v', '1', out.name], work)
    return out


def run(cmd, cwd, env=None):
    return subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, timeout=TIMEOUT)


def check(tex: Path, root: Path) -> list:
    spec = [l[2:].strip() for l in tex.read_text().splitlines() if l.startswith('%!')]
    fails = []
    with tempfile.TemporaryDirectory() as d:
        work = Path(d)
        for f in tex.parent.iterdir():
            (shutil.copytree if f.is_dir() else shutil.copy)(f, work / f.name)
        env = {**os.environ, 'TEXINPUTS': f'{root / "tex"}//:{HERE / "tex"}//:'}
        try:
            engine = next((l.split(':', 1)[1].strip() for l in spec if l.startswith('engine:')), 'uplatex')
            run([engine, '-interaction=nonstopmode', tex.name], work, env)
        except subprocess.TimeoutExpired:
            return [f'uplatex did not finish in {TIMEOUT} s (a loop?)']
        log = (work / tex.with_suffix('.log').name).read_text('utf-8', 'replace')
        dvi_path = work / tex.with_suffix('.dvi').name
        dvi = dvi_path.read_bytes() if dvi_path.exists() else b''
        pages = int(m.group(1)) if (m := re.search(r'Output written on .*?\((\d+) pages?', log)) else 0
        errors = len(re.findall(r'^! ', log, re.M))
        avi = None
        if dvi:
            r = run([str(DVIAVI), dvi_path.name], work)
            frames = re.search(rb': (\d+) frames', r.stderr)
            if r.returncode or not frames:
                fails.append('dviavi: ' + r.stderr.decode('utf-8', 'replace').strip().splitlines()[-1])
            elif int(frames.group(1)) != pages:
                fails.append(f'dviavi made {frames.group(1)} frames of {pages} pages')
            avi = dvi_path.with_suffix('.avi')
        want_errors = 0
        sp = specials(dvi)
        for line in spec:
            key, _, val = line.partition(':')
            val = val.strip()
            text, _, count = val.rpartition(' = ') if ' = ' in val else (val, '', '')
            if key == 'engine':
                pass
            elif key == 'pages' and int(val) != pages:
                fails.append(f'{pages} pages, not {val}')
            elif key == 'errors':
                want_errors = int(val)
            elif key == 'log':
                n = log.count(text)
                if (count and n != int(count)) or (not count and n == 0):
                    fails.append(f'log has `{text}\' {n} times' + (f', not {count}' if count else ''))
            elif key == 'special':
                n = sum(text in s for s in sp)
                if n != int(count):
                    fails.append(f'{n} specials with `{text}\', not {count}')
            elif key == 'compare' and avi and avi.exists():
                for p in (int(x) for x in val.split(',')):
                    diff = frame_diff(work, avi, p)
                    if diff > 0.1:
                        fails.append(f'frame {p}: {diff:.2f}% of the pixels differ from dvipdfmx')
        if errors != want_errors:
            first = re.search(r'^! .*', log, re.M)
            fails.append(f'{errors} errors, not {want_errors}' + (f' ({first.group(0)})' if first else ''))
    return fails


def main(args):
    root = HERE
    if args[:1] == ['--root']:
        root, args = Path(args[1]).resolve(), args[2:]
    base = root / 'tests' / 'tex'
    tests = sorted(base.glob('*/*.tex'))
    if args:
        tests = [t for t in tests if any(n in str(t.relative_to(base)) for n in args)]
    if not DVIAVI.exists():
        sys.exit(f'{DVIAVI} is missing: cargo build --release')
    bad = 0
    for t in tests:
        fails = check(t, root)
        name = t.relative_to(base)
        print(('ok    ' if not fails else 'FAIL  ') + str(name))
        for f in fails:
            print('      ' + f)
        bad += bool(fails)
    print(f'{len(tests) - bad}/{len(tests)} passed')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main(sys.argv[1:])
