# dviavi

DVI から AVI 動画を作る。**1 ページ = 1 フレーム**。音は sound.mix と `\special{dviavi: …}` から合成する。
中間画像ファイルは作らず、ffmpeg も組み込まない（外部音声ファイルのデコードにだけ、あれば `ffmpeg` コマンドを使う）。

詳しいリファレンス: [doc/dviavi.md](doc/dviavi.md)（ツール・TeX マクロ・special・UST ツール）、
[doc/sound-mix.md](doc/sound-mix.md)（sound.mix・MML・声と音素）。声の調整は `dviavi say` と `tools/voice-tuner.html`。

画面の大きさの紙のクラス（`avi.cls`）、animate 互換のアニメーション（`avianim.sty`）、声の定義と歌（`avivoice.sty`）は
[avipackages](https://github.com/Kiri-Nakuni/avipackages) リポジトリにある。

```
foo.tex ──(u)platex / euptex──▶ foo.dvi ──dviavi──▶ foo.avi
            dviavi.sty / dviavi.tex           MS-RLE 映像 + 16bit PCM 音声
```

## 使い方

```sh
make link-bin                         # ビルドして ~/.local/bin/dviavi にリンク
sudo env "PATH=$PATH" make install    # dviavi.tex・dviavi.sty を TEXMFLOCAL に（リンク）して mktexlsr
```

texmf-local に入れない場合:

```sh
cargo build --release
export TEXINPUTS=/path/to/dviavi/tex//:
uplatex foo.tex          # platex でも可。plain なら euptex / eptex
target/release/dviavi foo.dvi        # → foo.avi（-o で出力先指定）
```

必要なもの: TeX Live（`kpsewhich`、`pdftex.map`、`kanjix.map`）、FreeType。

## 出力形式

- 映像: MS-RLE（BI_RLE8、256 色パレット）。パレットは全フレームの色から作る（256 色以内なら色は正確、超えたら median cut）。
  前のフレームと同じところは delta で飛ばすので、変わらないフレームは 2 バイト。`keyint` フレームごとにキーフレーム。
- 音声: 16bit ステレオ PCM、`rate` Hz。長さは映像と同じ（はみ出した音は切れる）。
- AVI 1.0（2 GiB まで）。

## TeX 側

| 命令 | 意味 |
|---|---|
| `\dviaviSetup{fps=24000/1001, size=1920x1080, rate=48000, keyint=240, mix=sound.mix, paperwidth=160mm, paperheight=90mm}` | 出力形式。最初のフレームに書き出される（以後の変更は無効）。紙がそのまま画面になる |
| `\dviaviFrames{n}{本文}` | n フレーム。本文はフレームごとに組み直す（`\dviaviframe` に応じて変えられる） |
| `\dviaviFrame{本文}` | 1 フレーム |
| `\dviaviStill{n}{本文}` | 1 回だけ組んで n フレーム表示 |
| `\dviaviframe` | 出力済みフレーム数 = 今組んでいるフレームの番号（LaTeX では `\ReadonlyShipoutCounter`） |
| `\dviaviToFrames{t}` | 展開可能。`12f` `12` `1.5s` `150ms` `1:23.5` をフレーム数に（四捨五入） |
| `\dviaviLayout` | 本文の置き方（既定は中央）。再定義してよい |
| `\speak{ch=, tone=, speed=, pace=, text=, lang=}` | 発話（下記） |
| `\dviaviTone{楽器}{周波数}{長さ}` | 単音（`\dviaviTone{sine}{440}{1s}`） |
| `\dviaviMark{名前}` | このフレームに名前を付ける（sound.mix の `@名前+12f`） |
| `\dviaviMix{file}` | `\dviaviSetup{mix=file}` |

- LaTeX: 任意のクラス + `\usepackage[fps=30, paperwidth=160mm, paperheight=90mm]{dviavi}`（紙を画面にする avi クラスは avipackages リポジトリ）。
  色・画像・PGF は dvipdfmx ドライバで（`\usepackage[dvipdfmx]{xcolor}`）。フレームの外で指定した `\color` と `\pagecolor` はフレームに引き継がれる。
- plain: `\input dviavi`（e-TeX 拡張が必要なので `euptex` / `eptex`）。expl3 を使うので `expl3-generic` を読み込む（1 回 10 秒ほどかかる）。
- 音の命令は、繰り返しフレームの最初の 1 枚でだけ働く。フレームとフレームの間に書いた場合は、次に出力されるページに付く。
- ページの延長はしない。音がページより長くても、そのまま鳴り続ける。

### `\speak`

```tex
\speak{ch=example, tone=normal, speed=.9, pace=1., text=KoNnNiTiWa}
```

- `ch`: sound.mix の `voice`（なければ既定の声）。`tone`: その voice の `tone`（既定 `normal`）
- `speed`: 周波数の倍率。`pace`: 1 モーラの長さ（単位なし = voice の `mora` に掛ける倍率、`6f` / `150ms` = 絶対値）
- `text`: ASCII ローマ字。**大文字がモーラの先頭**（`Ko Nn Ni Ti Wa`）。`'` アクセント核、`/` 句切り、`_` ポーズ、`-` 長音。
  `lang=en` なら ARPAbet の音節を空白で区切る（`D.EY Z.IY`）
- 子音・母音・ん（後ろの音に同化）を出す。音素は [doc/sound-mix.md](doc/sound-mix.md#音素とモーラ)

## sound.mix

```
% コメント
inst lead { wave=square duty=0.25 adsr=5ms,80ms,0.6,120ms gain=-6dB detune=0c,7c lpf=3k vib=5Hz,10c }
voice example { f0=200 mora=150ms formant=1.1 breath=0.03 gain=0.5 vib=5.5Hz,12c
  tone normal { f0x=1 range=4st }
  tone happy  { f0x=1.15 range=7st } }

track bgm gain=-6dB pan=-0.3
  @0f lead t150 o5 l8 c d e g <c e g>4 r4
    o+ c4                          % 次の @ までは直前のクリップの続き
  @intro+12f file "boom.wav" gain=-3dB
track vocal
  @2s example t120 o4 "KoNnNiTiWa" c d e f g2
```

- 時刻: `12f` / 数字だけ（フレーム）/ `1.5s` / `150ms` / `1:23.5` / `マーク名` / `マーク名+12f` / `マーク名-1s`
- `inst` のパラメータ: `wave`（`sine` `square` `triangle` `saw` `noise`）、`duty`、`adsr=a,d,s,r`、`gain`（`-6dB` か倍率）、
  `detune`（セント、複数で重ねる）、`lpf`、`vib=速さ,深さ`。`sine` などの波形名は定義なしでそのまま楽器として使える
- `voice` の名前をクリップに使うと、MML の音程で歌う。`"…"` の歌詞は 1 モーラずつ後続の音符に割り当てる
- MML: `c d e f g a b`（`+`/`#` `-`）、長さ `4`、付点 `.`、タイ `&`、休符 `r`、和音 `<c e g>4`、
  `t` テンポ、`o4` / `o+` / `o-` オクターブ、`l` 既定の長さ、`v0-127` ベロシティ、`q1-8` ゲート、`|` は無視
- 外部ファイル: 同じサンプリング周波数の WAV は直接読む。それ以外は `ffmpeg` をパイプで呼んでデコードする
- 全体がクリップしそうなときはソフトクリップして警告する

## UST（UTAU）から作る: `tools/ust2dviavi.py`

```sh
python3 tools/ust2dviavi.py mix --offset 34b Main.ust:triangle Harmony.ust:sine:-6dB >> sound.mix
python3 tools/ust2dviavi.py script Main.ust > lyrics.txt      # 歌詞の SKK キー列（ひらがな入力の雛形）
python3 tools/ust2dviavi.py timeline --offset 34b Main.ust lyrics.txt > timeline.tex
```

- `mix`: UST の音符を MML のクリップにする（20 tick 単位の三連符もタイで正確に表す）
- `script`: 1 行 1 フレーズ、1 音符 1 トークン。大文字で変換開始、`+` がスペースキー、`=` はキーなしの音符、`!keys` は音符なしのキー
- `timeline`: 各キーを音符の発音時刻に合わせ、texskk の `\SKKinputto\lyricbuf{キー列}\lyricframes{フレーム数}` の列を出力する
  （フレーズの区切りに `\lyriccommit`・`\lyricnext`。キー列の書き方は [doc/dviavi.md](doc/dviavi.md)）
- `--offset` は UST の先頭の動画内の時刻（`11.1475s`、`34b` = 拍、`267f`）

## テスト

```sh
cargo test                     # Rust
python3 tests/tex/run.py       # TeX（plain・dviavi.sty）。cargo build --release の後
python3 tests/tex/run.py --root ../avipackages   # ほかのリポジトリのテスト（その tests/tex/）
```

## 制限

- 縦組（pTeX の `dir`）は位置と回転だけ対応。縦書き用の字形（句読点など）への置き換えはしない
- 画像は dvipdfmx の `pdf:image`（graphicx）を、ffmpeg（PNG・JPEG など）と mutool / pdftocairo（PDF）で読む。
  PDF の文字・インライン画像・PostScript の special は描かない（[doc/dviavi.md](doc/dviavi.md#dvipdfmx-の-special)）
- フォントは `pdftex.map`（Type 1）と `kanjix.map`（和文 OpenType）で解決できるもの。JIS X 0213 の拡張文字は未対応

## ライセンスと謝辞

MIT ライセンス（[LICENSE](LICENSE)）。

DVI 形式を設計した David Fuchs に感謝します。
