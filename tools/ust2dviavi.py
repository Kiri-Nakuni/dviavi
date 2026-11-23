#!/usr/bin/env python3
"""ust2dviavi.py -- UTAU scores (UST) to dviavi material.

    python3 ust2dviavi.py mix [--offset T] [--lyrics] FILE.ust:INST[:GAIN] ...
        sound.mix tracks playing the UST notes with instruments, or with
        --lyrics singing them with voices (each note gets its lyric's mora)
    python3 ust2dviavi.py script FILE.ust
        a lyric key script: one phrase per line (phrases end at rests),
        one token per note, typed as plain hiragana; edit it to add SKK
        conversions (capitals, `+` for the space key)
    python3 ust2dviavi.py timeline [--offset T] [--fps N/D] [--frames N | --tail T] FILE.ust SCRIPT
        TeX timeline for a texskk lyric display: \\lyricintro{frames},
        then \\SKKinputto\\lyricbuf{keys}\\lyricframes{frames} per state;
        \\lyriccommit before the frames of a phrase's last state (all its keys
        typed) and \\lyricnext between phrases

T is the time of the UST's start in the video (`11.1475s`, `34b` = beats, `267f`);
for mix it may also be `mark+T`, relative to \\dviaviMark{mark}.

Script syntax (one phrase per line, `#` starts a comment):
    token      keys typed for one note: the key completing the kana (the last
               vowel, or the `n` of ん) is typed at the note's onset, keys before
               it just before, keys after it half way to the next note.
               `+` is the space key (conversion), `\\+` a literal `+`, `^j` C-j (confirm).
    =          a note with no keys (a held or repeated vowel)
    !keys      keys that take no note (typed half way to the next note)
"""
import argparse
import re
import sys
from fractions import Fraction

TICKS_PER_QUARTER = 480

# ---------------------------------------------------------------- UST


class Note:
    def __init__(self, start, length, lyric, pitch, tempo):
        self.start = start      # seconds from the UST start
        self.length = length    # seconds
        self.ticks = 0
        self.lyric = lyric
        self.pitch = pitch
        self.tempo = tempo

    @property
    def rest(self):
        return self.lyric in ('R', 'r', '')


def read_ust(path):
    raw = open(path, 'rb').read()
    for enc in ('utf-8', 'cp932'):
        try:
            text = raw.decode(enc)
            break
        except UnicodeDecodeError:
            continue
    tempo = 120.0
    sections = []
    cur = None
    for line in text.splitlines():
        m = re.match(r'\[#(\w+)\]', line)
        if m:
            cur = {'#': m.group(1)}
            sections.append(cur)
        elif cur is not None and '=' in line:
            k, v = line.split('=', 1)
            cur[k] = v
    notes = []
    t = 0.0
    for s in sections:
        if s['#'] == 'SETTING' and 'Tempo' in s:
            tempo = float(s['Tempo'])
        if not s['#'].isdigit():
            continue
        if 'Tempo' in s:
            tempo = float(s['Tempo'])
        ticks = int(s.get('Length', '0'))
        sec = ticks / TICKS_PER_QUARTER * 60.0 / tempo
        n = Note(t, sec, s.get('Lyric', 'R'), int(s.get('NoteNum', '60')), tempo)
        n.ticks = ticks
        notes.append(n)
        t += sec
    return notes


def parse_offset(s, fps, tempo):
    s = s.strip()
    if s.endswith('ms'):
        return float(s[:-2]) / 1000
    if s.endswith('s'):
        return float(s[:-1])
    if s.endswith('b'):
        return float(s[:-1]) * 60.0 / tempo
    if s.endswith('f'):
        s = s[:-1]
    return float(s) / fps

# ---------------------------------------------------------------- mix

# note values (as 1/n of a whole note) used to spell arbitrary tick counts
LENGTHS = sorted({1, 2, 4, 8, 16, 32, 64, 3, 6, 12, 24, 48, 96, 192},
                 key=lambda n: Fraction(1, n), reverse=True)


def spell_length(ticks):
    """Tick count as tied MML lengths: 340 -> ['8', '24', '96']."""
    whole = TICKS_PER_QUARTER * 4
    rest = Fraction(ticks, whole)
    out = []
    for n in LENGTHS:
        while rest >= Fraction(1, n):
            out.append(str(n))
            rest -= Fraction(1, n)
    if rest:
        raise ValueError('length %d ticks cannot be spelt' % ticks)
    return out


NAMES = ['c', 'c+', 'd', 'd+', 'e', 'f', 'f+', 'g', 'g+', 'a', 'a+', 'b']


def lyric_mora(lyric):
    """A UST lyric as a mora for a dviavi voice (`か` -> Ka, `ん` -> Nn,
    `ー` -> -); None when it is not kana (the previous vowel goes on)."""
    l = lyric.strip()
    # CV/VCV spellings: `- か`, `a か` (the mora is the last word)
    l = l.split()[-1] if l.split() else ''
    l = l.rstrip('↑↓→←')
    if l in ('-', 'ー', '+'):
        return '-' if l != '+' else None
    if not l or not all('\u3041' <= c <= '\u30ff' for c in l):
        return None
    hira = ''.join(chr(ord(c) - 0x60) if 'ァ' <= c <= 'ヶ' else c for c in l)
    if hira == 'っ':
        return 'Q'
    r = romaji(hira)
    if not re.fullmatch(r'[a-z]+', r):
        return None
    return r[0].upper() + r[1:]


def mml_of(notes, lyrics=False):
    words = []
    tempo = None
    for n in notes:
        if n.tempo != tempo:
            tempo = n.tempo
            words.append('t%g' % tempo)
        lens = spell_length(n.ticks)
        if not lens:
            continue
        if n.rest:
            words.append(' '.join('r' + l for l in lens))
        else:
            octave = n.pitch // 12 - 1
            note = 'o%d%s%s' % (octave, NAMES[n.pitch % 12], '&'.join(lens))
            mora = lyric_mora(n.lyric) if lyrics else None
            words.append('"%s" %s' % (mora, note) if mora else note)
    lines = []
    line = ''
    for w in words:
        if len(line) + len(w) > 90:
            lines.append(line)
            line = ''
        line += (' ' if line else '') + w
    lines.append(line)
    return lines


def cmd_mix(args):
    for spec in args.tracks:
        parts = spec.split(':')
        path, inst = parts[0], parts[1] if len(parts) > 1 else 'triangle'
        gain = parts[2] if len(parts) > 2 else '0dB'
        notes = read_ust(path)
        # `mark+T`: relative to a \dviaviMark, so frames added before it do not matter
        m = re.match(r'([A-Za-z_]\w*)\+(.+)$', args.offset)
        mark, rel = (m.group(1) + '+', m.group(2)) if m else ('', args.offset)
        offset = parse_offset(rel, 24000 / 1001, notes[0].tempo if notes else 120)
        name = re.sub(r'\W', '_', path.rsplit('/', 1)[-1].rsplit('.', 1)[0])
        print('track %s gain=%s' % (name, gain))
        lines = mml_of(notes, args.lyrics)
        print('  @%s%.4fs %s q8 %s' % (mark, offset, inst, lines[0]))
        for l in lines[1:]:
            print('    ' + l)

# ---------------------------------------------------------------- romaji

KANA = {}


def _kana_table():
    rows = {
        '': 'あいうえお', 'k': 'かきくけこ', 's': 'さしすせそ', 't': 'たちつてと',
        'n': 'なにぬねの', 'h': 'はひふへほ', 'm': 'まみむめも', 'r': 'らりるれろ',
        'g': 'がぎぐげご', 'z': 'ざじずぜぞ', 'd': 'だぢづでど', 'b': 'ばびぶべぼ',
        'p': 'ぱぴぷぺぽ',
    }
    for c, kana in rows.items():
        for v, k in zip('aiueo', kana):
            KANA[k] = c + v
    KANA.update({'や': 'ya', 'ゆ': 'yu', 'よ': 'yo', 'わ': 'wa', 'を': 'wo', 'ん': 'nn',
                 'ゔ': 'vu', 'ー': '-', 'っ': 'xtu',
                 'ぁ': 'xa', 'ぃ': 'xi', 'ぅ': 'xu', 'ぇ': 'xe', 'ぉ': 'xo'})
    small = {'ゃ': 'ya', 'ゅ': 'yu', 'ょ': 'yo'}
    for k, r in list(KANA.items()):
        if len(k) == 1 and r.endswith('i') and len(r) == 2:
            for s, y in small.items():
                KANA[k + s] = r[0] + y
    # common two-letter spellings
    KANA.update({'しゃ': 'sya', 'しゅ': 'syu', 'しょ': 'syo', 'ちゃ': 'tya', 'ちゅ': 'tyu',
                 'ちょ': 'tyo', 'じゃ': 'zya', 'じゅ': 'zyu', 'じょ': 'zyo',
                 'てぃ': 'thi', 'でぃ': 'dhi', 'ふぁ': 'fa', 'ふぃ': 'fi', 'ふぇ': 'fe', 'ふぉ': 'fo'})


_kana_table()


def romaji(lyric):
    out = ''
    i = 0
    while i < len(lyric):
        two = lyric[i:i + 2]
        if two in KANA:
            out += KANA[two]
            i += 2
        elif lyric[i] in KANA:
            out += KANA[lyric[i]]
            i += 1
        else:
            out += lyric[i]
            i += 1
    return out


def cmd_script(args):
    notes = read_ust(args.ust)
    print('# lyric key script for %s: one phrase per line, one token per note' % args.ust)
    print('# capitals start a conversion, + is the space key, = a note without keys,')
    print('# !keys keys without a note (see ust2dviavi.py)')
    # a line ends at a rest of a beat or more, or at a long note once it is long
    phrase = []
    for i, n in enumerate(notes):
        if n.rest:
            if phrase and (n.ticks >= TICKS_PER_QUARTER or len(phrase) >= 8):
                print(' '.join(phrase))
                phrase = []
            continue
        phrase.append(romaji(n.lyric))
        if len(phrase) >= 12 and n.ticks >= TICKS_PER_QUARTER or len(phrase) >= 20:
            print(' '.join(phrase))
            phrase = []
    if phrase:
        print(' '.join(phrase))


# ---------------------------------------------------------------- timeline


def completing_index(token):
    """Index of the key that completes the note's kana."""
    for i in range(len(token) - 1, -1, -1):
        if token[i] in 'aiueoAIUEO':
            return i
    for i in range(len(token) - 1, -1, -1):
        if token[i] in 'nN':
            return i
    return len(token) - 1


def tex_keys(keys):
    """Keys for texskk's \\SKKinputto (read with its own catcodes)."""
    out = ''
    for k in keys:
        if k == '\n':
            out += '\\SKKCtrl{j}'
        elif k == ' ':
            out += ' '
        elif k in '{}%\\':
            out += '\\' + k
        else:
            out += k
    return out


def split_tokens(line):
    """Whitespace-separated tokens; `+` becomes a space key, `\\+` a plus."""
    toks = []
    for t in line.split():
        toks.append(t.replace('\\+', '\0').replace('+', ' ').replace('\0', '+').replace('^j', '\n'))
    return toks


def cmd_timeline(args):
    num, den = (int(x) for x in args.fps.split('/'))
    fps = num / den
    notes = [n for n in read_ust(args.ust) if not n.rest]
    offset = parse_offset(args.offset, fps, notes[0].tempo if notes else 120)
    phrases = []
    for raw in open(args.script, encoding='utf-8'):
        line = raw.split('#', 1)[0].strip()
        if line:
            phrases.append(split_tokens(line))

    # (time, phrase index, key) for every key
    keys = []
    ni = 0
    last_t = 0.0
    for pi, toks in enumerate(phrases):
        for tok in toks:
            if tok.startswith('!'):
                nxt = notes[ni].start + offset if ni < len(notes) else last_t + 0.2
                t = (last_t + nxt) / 2
                for k in tok[1:]:
                    keys.append((t, pi, k))
                last_t = t
                continue
            if ni >= len(notes):
                sys.exit('script has more notes than %s (phrase %d)' % (args.ust, pi + 1))
            n = notes[ni]
            ni += 1
            onset = n.start + offset
            nxt = notes[ni].start + offset if ni < len(notes) else onset + n.length
            if tok == '=':
                continue
            c = completing_index(tok)
            lead, comp, trail = tok[:c], tok[c], tok[c + 1:]
            # lead keys just before the onset, not before the previous key
            window = min(0.15, max(0.0, onset - last_t))
            for j, k in enumerate(lead):
                keys.append((onset - window * (len(lead) - j) / (len(lead) + 1), pi, k))
            keys.append((onset, pi, comp))
            tt = onset + (nxt - onset) / 2
            for k in trail:
                keys.append((tt, pi, k))
            last_t = tt if trail else onset
    if ni != len(notes):
        print('ust2dviavi: warning: the script covers %d of %d notes' % (ni, len(notes)), file=sys.stderr)

    if args.frames:
        total = args.frames
    elif args.tail is not None:
        # until `tail` after the last key
        total = int((keys[-1][0] + parse_offset(args.tail, fps, notes[0].tempo)) * fps) + 1
    else:
        total = int((notes[-1].start + notes[-1].length + offset) * fps) + 1
    first_frame = int(keys[0][0] * fps) if keys else total
    print('% generated by ust2dviavi.py timeline; do not edit')
    print('\\lyricintro{%d}' % first_frame)
    f = first_frame
    k = 0
    full = {}  # every key of each phrase
    for _, pi, key in keys:
        full[pi] = full.get(pi, '') + key
    runs = []  # [phrase, keys, frames]
    phrase_keys = ''
    cur_phrase = -1
    while f < total:
        t = (f + 0.5) / fps
        while k < len(keys) and keys[k][0] <= t:
            _, pi, key = keys[k]
            if pi != cur_phrase:
                cur_phrase = pi
                phrase_keys = ''
            phrase_keys += key
            k += 1
        if runs and runs[-1][:2] == [cur_phrase, phrase_keys]:
            runs[-1][2] += 1
        else:
            runs.append([cur_phrase, phrase_keys, 1])
        f += 1
    prev = None
    committed = set()
    for pi, ks, n in runs:
        if prev is not None and pi != prev:
            if prev not in committed:
                # its last keys fell in the frame where the next phrase starts
                print('\\SKKinputto\\lyricbuf{%s}\\lyriccommit' % tex_keys(full[prev]))
                committed.add(prev)
            print('\\lyricnext')
        prev = pi
        commit = ''
        if ks == full.get(pi):
            commit = '\\lyriccommit'
            committed.add(pi)
        print('\\SKKinputto\\lyricbuf{%s}%s\\lyricframes{%d}' % (tex_keys(ks), commit, n))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest='cmd', required=True)
    m = sub.add_parser('mix')
    m.add_argument('--offset', default='0s')
    m.add_argument('--lyrics', action='store_true', help='sing: write the UST lyrics as morae (for voices)')
    m.add_argument('tracks', nargs='+')
    s = sub.add_parser('script')
    s.add_argument('ust')
    t = sub.add_parser('timeline')
    t.add_argument('--offset', default='0s')
    t.add_argument('--fps', default='24000/1001')
    t.add_argument('--frames', type=int, default=0, help='total frames (default: to the end of the UST)')
    t.add_argument('--tail', default=None, help='end this long after the last key (`5s`, `24f`, `2b`)')
    t.add_argument('ust')
    t.add_argument('script')
    args = p.parse_args()
    {'mix': cmd_mix, 'script': cmd_script, 'timeline': cmd_timeline}[args.cmd](args)


if __name__ == '__main__':
    main()
