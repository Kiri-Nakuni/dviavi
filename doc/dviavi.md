# dviavi リファレンス

DVI を動画（AVI）の素材として読むツールと、そのための TeX マクロのリファレンス。
音の記述言語 sound.mix は [sound-mix.md](sound-mix.md) を参照。

- [考え方](#考え方)
- [コマンド](#コマンド)
- [出力形式](#出力形式)
- [TeX 側のマクロ](#tex-側のマクロ)
- [special](#special)
- [フォント](#フォント)
- [色](#色)
- [dviavi say](#dviavi-say)
- [dvipdfmx の special](#dvipdfmx-の-special)
- [UST から作る: tools/ust2dviavi.py](#ust-から作る-toolsust2dviavipy)
- [制限](#制限)

## 考え方

- **1 ページ = 1 フレーム**。何フレーム見せるか、何を動かすかは TeX のマクロが決める。dviavi はページを順に絵にするだけ。
- **時間の基準はフレーム**。音もフレーム（＝ページ）を起点に置く。秒で書いたものはフレームレートで換算する。
- 出力形式・音・目印は `\special{dviavi: …}` で DVI に書き込む。形式の情報源は TeX 側だけで、dviavi のコマンドラインでは変えない。
- 中間の画像ファイルは作らない。ffmpeg も組み込まない（外部の音声ファイルをデコードするときだけ `ffmpeg` コマンドを使う）。

## コマンド

```sh
dviavi [-o OUTPUT.avi] INPUT[.dvi]
```

- `INPUT` に拡張子がなく、そのファイルがなければ `.dvi` を補う。
- 出力の既定は入力と同じ名前の `.avi`。
- sound.mix は `config` special の `mix=`（DVI のあるディレクトリから見たパス）で指定する。

処理の流れ: special の読み取り → 音の合成 → 全ページを描いて色を集める → 256 色パレットを作る → もう一度描いて符号化。
同じ内容のページが続く間は描き直さないので、静止画を長く見せても速い。

ビルド:

```sh
cargo build --release      # target/release/dviavi。FreeType と TeX Live（kpsewhich）が必要
```

## 出力形式

| | |
|---|---|
| コンテナ | RIFF AVI 1.0（インデックス付き）。2 GiB を超えるとエラー |
| 映像 | MS-RLE（BI_RLE8）、256 色パレット 1 枚、ボトムアップ |
| 音声 | 16 bit リニア PCM、ステレオ、`rate` Hz（既定 48000） |
| 長さ | ページ数 × フレーム時間。音声も同じ長さで切る |

- **パレット**: 全フレームの色を数え、256 色以内ならそのまま、超えたら median cut で減色する（メッセージが出る）。
  文字（アンチエイリアスの階調）と少数の色なら、色は正確に出る。
- **差分フレーム**: 前のフレームと同じ画素は RLE8 の delta で飛ばす。変化のないフレームは 2 バイト。
- **キーフレーム**: `keyint` フレームごと（既定 240 = 約 10 秒）。シーク用。
- **フレームレート**: `fps=24000/1001` は AVI の rate/scale にそのまま入るので誤差がない。
- **音量**: 全体の振幅が 0.8 を超えたところをなめらかに抑え（ソフトクリップ）、1.0 を超えていたら警告する。

画面の大きさは `size`（既定 1920x1080、奇数は偶数に切り上げ）。紙（`paper=`、なければ `papersize` special）の
幅が画面の幅になるように拡大する。縦横比が違えば警告して幅に合わせる。紙の指定がまったくなければ、
幅 160mm で画面と同じ縦横比の紙とみなす。DVI の原点は紙の左上から 1 true inch の位置（dvips・dvipdfmx と同じ）。

## TeX 側のマクロ

| ファイル | 用途 |
|---|---|
| `tex/dviavi.tex` | 本体。plain の e-pTeX / e-upTeX（`eptex`, `euptex`）では `\input dviavi` |
| `tex/dviavi.sty` | (u)pLaTeX 用。`\usepackage[fps=30,size=1280x720]{dviavi}`（オプションは `\dviaviSetup` のキー） |

画面の大きさの紙のクラス（`avi.cls`）、animate 互換のアニメーション（`avianim.sty`）、声の定義と歌（`avivoice.sty`）は
別のリポジトリ [avi](https://github.com/Kiri-Nakuni/avi) にある。

e-TeX 拡張が必要（`ptex`/`uptex` コマンドでは読み込みを止める）。マクロは expl3 で書いてあり、plain では `expl3-generic` を読み込む
（LaTeX は expl3 を最初から持っているので負担はないが、plain ではコンパイルのたびに 10 秒ほどかかる）。
plain の expl3 の出力先（backend）は dvipdfmx にする（dviavi が読む形式）。

ほかのパッケージ向けの expl3 の関数:

| 関数 | 内容 |
|---|---|
| `\dviavi_sound:n {special}` | 音の special。作っているフレームに入れる（フレームの間なら次のフレーム）。`\dviaviFrames`・`\dviaviStill` の繰り返しでは最初の 1 枚だけ |
| `\dviavi_special:n {special}` | 同じところに、いつでも入れる |
| `\dviavi_fps:` | フレームレート（fp の式、展開可能） |

### 形式

```tex
\dviaviSetup{fps=24000/1001, size=1920x1080, rate=48000, keyint=240,
             mix=sound.mix, paperwidth=160mm, paperheight=90mm}
\dviaviMix{sound.mix}                      % = \dviaviSetup{mix=sound.mix}
```

| キー | 既定値 | 意味 |
|---|---|---|
| `fps` | `24000/1001` | フレームレート。`N/D` か整数 |
| `size` | `1920x1080` | 画面の画素数 |
| `rate` | `48000` | 音声のサンプリング周波数 |
| `keyint` | `240` | キーフレームの間隔（フレーム） |
| `mix` | なし | sound.mix のファイル |
| `paperwidth`, `paperheight` | `160mm`, `90mm` | 紙＝画面の大きさ。LaTeX では `\paperwidth`/`\paperheight` そのもの |

値は最初のフレームを出力するときに `dviavi: config` special として書き出される。それより後の `\dviaviSetup` はエラー。
キーと値の中の空白は無視する。

### フレーム

```tex
\dviaviFrame{本文}                 % 1 フレーム
\dviaviFrames{n}{本文}             % n フレーム。本文はフレームごとに組み直す
\dviaviStill{n}{本文}              % 1 回だけ組んで n フレーム表示
\dviaviframe                       % 出力済みのフレーム数 = 今組んでいるフレームの番号（0 から）
```

- `\dviaviFrames` の本文は毎回組まれるので、`\dviaviframe` を使えばフレームごとに変わる絵（点滅、時計など）が作れる。
- `\dviaviStill` は同じ絵を繰り返す。dviavi は同じページを描き直さず、差分 0 のフレームとして書く。
- 本文は `\dviaviLayout{…}` で紙に置かれる。既定は中央。`\renewcommand\dviaviLayout[1]{…}` で自由に変えられる
  （引数を紙の大きさの箱に入れる。左上が紙の左上）。
- フレームは、それを組んだ場所の色で始まる（LaTeX の `\color`）。フレームの外の `\pagecolor` は次のページに付く。
- `\hoffset`, `\voffset` の値に関係なく、フレームの左上が紙の左上になる。
- LaTeX の通常のページも 1 ページ = 1 フレームとして数えられる（`\dviaviframe` は LaTeX では `\ReadonlyShipoutCounter`）。

### 時間

```tex
\dviaviToFrames{1.5s}     % 展開可能。フレーム数（四捨五入）に
```

`12f`、`12`（フレーム）、`1.5s`、`150ms`、`1:23.5`（分:秒）。秒は 16383 まで（TeX の寸法の範囲）。
現在の `fps` で換算するので、`\dviaviSetup` の後で使う。

### 音

```tex
\speak{ch=example, tone=normal, speed=.9, pace=1., text=KoNnNiTiWa}
\dviaviTone{sine}{440}{1s}         % 楽器または波形, 周波数, 長さ
\dviaviMark{intro}                 % このフレームに名前を付ける（sound.mix の @intro+12f）
```

- 音はその命令があるページ（フレーム）の時刻に始まる。ページは延長しない。重なった音は加算する。
- `\dviaviFrames`/`\dviaviStill` の繰り返しでは、最初の 1 枚でだけ働く。
- フレームとフレームの間（外側の垂直モード）に書いたものは、次に出力されるページに付く。

`\speak` のキー:

| キー | 既定値 | 意味 |
|---|---|---|
| `ch` | `default` | sound.mix の `voice`。未定義なら既定の声（警告） |
| `tone` | `normal` | その voice の `tone`。未定義なら f0x=1, range=4st |
| `speed` | `1` | 周波数（声の高さ）の倍率 |
| `pace` | `1` | 1 モーラの長さ。単位なしは voice の `mora` に掛ける倍率、`6f` / `150ms` / `0.2s` は絶対値 |
| `text` | （必須） | 読み。ASCII のローマ字で、**大文字がモーラの先頭**（`lang=en` なら ARPAbet の音節を空白で区切る） |
| `lang` | 声の `lang` | `ja` か `en` |

書いたキーだけを、書いた順に special に書く。空白を含む値は `{}` で囲んで渡す。

`text` の記号: `'` 直前のモーラがアクセント核（そこまで高く、後は低い）、`/` 句の区切り（音程を戻す）、
`_` 1 モーラの休み、`-` 直前の母音を伸ばす。`Q`（っ）は無音。音素は [sound-mix.md](sound-mix.md#音素とモーラ)。

歌う（`\sing`）・声を文書の中で定義するには **avivoice.sty**（[avivoice.md](https://github.com/Kiri-Nakuni/avi/blob/main/doc/avivoice.md)）を使う。

## special

TeX 側のマクロが書くものだが、直接書いてもよい。

```
dviavi: config fps=24000/1001 size=1920x1080 rate=48000 keyint=240 mix=sound.mix paper=160mm,90mm
dviavi: mark name=intro
dviavi: speak ch=example tone=normal speed=.9 pace=1. text=KoNnNiTiWa
dviavi: tone inst=sine freq=440 dur=2s amp=0.5
dviavi: speak ch=daisy lang=en text={D.EY Z.IY}
dviavi: sing ch=daisy lang=en gain=0.8 mml={t150 "D.EY Z.IY" o4 d2. o3 b2.}
dviavi: mixdef voice daisy { engine=square lang=en phoneme k { dur=25ms } }
papersize=160mm,90mm
```

- `config` は**最初のページ**のものだけが有効（後のページのものは警告して無視）。`paper=` は `papersize` に優先する。
- 寸法の単位: `pt bp mm cm in pc dd cc sp Q`、`true` を付けると拡大率（`\mag`）の影響を受けない。
- `tone`: `inst`（または `wave`）は sound.mix の楽器か波形名、`freq` は `440` / `440Hz` / `2k`、
  `dur` は時間、`amp` を書くと楽器の `gain` の代わりにその振幅で鳴らす。
- `speak`・`sing` の `lang`: `ja` か `en`（なければ声の `lang`）。
- `sing`: その声で MML（歌詞付き）を、そのページの時刻から歌う（sound.mix の声のクリップと同じ。`gain` はその音量）。
- `mixdef`: 残りを sound.mix の文として読む（`voice`・`phoneme`・`inst` の定義）。どのページのものも、
  sound.mix の後にページ順に足して読むので、同じ名前の sound.mix の定義より優先する。
- 値に空白を含めるときは `key="a b"` か `key={a b}`（`{}` の中は入れ子にでき、`"` も書ける）。
- 知らない `dviavi:` の命令は警告して無視する。

## フォント

TeX Live の設定をそのまま使い、dvipdfmx と同じフォントで描く。ファイルはすべて `kpsewhich` で探す。

- **欧文**: `pdftex.map` で TFM 名から Type 1 フォントへ。`.enc` による再符号化、`SlantFont`, `ExtendFont` に対応。
  `.enc` のないものはフォント自身の符号化で引く。
- **和文**: dvipdfmx の `kanjix.map` で JFM 名から OpenType へ。
  CMap `H` / `V` / `2004-H` / `2004-V` は JIS コード（pTeX）、`UniJIS*` は Unicode（upTeX）として扱う。
  `:n:`（TTC の番号）、`!`、`-s`（斜体）、`-e`（横幅）に対応。
- **仮想フォント**（VF）は展開して描く（pLaTeX の `jis` → `rml`、upLaTeX の `upjisr-h` → `uprml-h` など）。
- 字幅は TFM / JFM の値を使う（TeX が組んだ位置どおりに置く）。
- 見つからないフォントは警告して、その文字を描かない。

## 色

| special | 出すもの |
|---|---|
| `color push MODEL …` / `color pop` / `color MODEL …` | dvips・dvipdfmx 形式の色スタック |
| `background MODEL …` | 背景色（そのページ以降。`\pagecolor`） |
| `pdf:bcolor [...]` / `pdf:ecolor` / `pdf:bgcolor [...]` | dvipdfmx の旧形式 |
| `pdfcolorstack N push (...)` / `pop` / `set (...)` | dvipdfmx の色スタック（塗りの色を使う） |

MODEL は `rgb`, `cmyk`, `gray`, `hsb`。色名（`Red` など）は未対応（xcolor は数値で書き出すので通常は問題ない）。
色スタックと背景はページをまたいで引き継ぐ（dvips と同じ）。LaTeX では `\usepackage[dvipdfmx]{xcolor}` を使う。

## dviavi say

```sh
dviavi say [--mix FILE] [--voice NAME] [--tone T] [--speed X] [--pace P] [--rate HZ] [--lang ja|en] [-o OUT.wav] [--play] TEXT
dviavi say [--mix FILE] [--voice NAME] ... --sing 'MML with "lyrics"'
```

mix ファイルの声で、話す（TEXT は `\speak` と同じローマ字）か歌う（`--sing` の MML、歌詞は `"…"`）のを WAV にする（既定 `say.wav`、16 bit モノラル）。
`--lang` は TEXT と歌詞の書き方（既定はその声の `lang`）。動画を作らずに声と音素を調整するためのもの。`--play` で書き出した後に再生する（`ffplay`、なければ `aplay`）。
声と `phoneme` の書き方は [sound-mix.md](sound-mix.md#phoneme音素の調整)。ブラウザで調整するなら `tools/voice-tuner.html`。

## dvipdfmx の special

dviavi は dvipdfmx の special のうち絵になるものを解釈する。graphicx・xcolor・PGF/TikZ を dvipdfmx ドライバで使えば
（avi クラスなら自動）、dvipdfmx で PDF にしたときとほぼ同じ絵になる。

| special | 内容 |
|---|---|
| `pdf:code`, `pdf:literal [direct]`, `pdf:content` | PDF の描画演算子（下） |
| `pdf:bcontent` / `pdf:econtent` | 現在位置を原点にするブロック（中の文字も正しい位置に置く） |
| `pdf:btrans [matrix a b c d e f / rotate θ / scale s / xscale / yscale]` / `pdf:etrans` | 現在位置まわりの変換（graphicx の回転・拡大） |
| `x:gsave`, `x:grestore`, `x:scale sx sy`, `x:rotate θ` | 同上（graphicx の `\scalebox`） |
| `pdf:bxobj @name …` / `pdf:exobj` / `pdf:uxobj @name` | フォーム XObject（PGF のシェーディングなど）。中身を記録し、使う位置で描く |
| `pdf:put @resources <<…>>`, `pdf:obj @name …`, `pdf:put @name …` | リソース（ExtGState・Shading）と名前付きオブジェクト |
| `pdf:image [@name] [hide] [page n] [bbox …] [width w] [height h] (file)` | 画像（PNG・JPEG・BMP・GIF・TIFF・WebP は ffmpeg、PDF は mutool か pdftocairo） |
| `PSfile="file" …` | EPS（Ghostscript） |
| 色: `color`, `background`, `pdf:bcolor`/`ecolor`/`bgcolor`, `pdfcolorstack` | [色](#色) |

描画演算子: `q Q cm w J j M d gs`（ExtGState の `CA` `ca` `LW`）、`g G rg RG k K sc SC scn SCN`、
`m l c v y h re`、`S s f F f* B B* b b* n W W*`、`sh`（ShadingType 2・3、関数 Type 2・3）。
文字は現在の変換行列に従う（TikZ の `transform shape` や `\rotatebox` の中の文字も回る）。

描かないもの（警告を出す）: パターン塗り、ソフトマスク（PGF のフェーディング）、関数 Type 0・4 のシェーディング、
PDF の文字演算子（`BT`…`ET`）、インライン画像、PostScript の special（`ps:`、`"`）。
リンク・しおり・文書情報などの絵にならない special は何もしない。

## UST から作る: tools/ust2dviavi.py

UTAU の楽譜（UST、Shift_JIS か UTF-8）から、音と歌詞入力のタイミングを作る。

```sh
python3 tools/ust2dviavi.py mix --offset 34b Main.ust:triangle Harmony.ust:sine:-6dB >> sound.mix
python3 tools/ust2dviavi.py script Main.ust > lyrics.txt
python3 tools/ust2dviavi.py timeline --offset 34b [--fps 24000/1001] [--frames N | --tail 5s] Main.ust lyrics.txt > timeline.tex
```

- `--offset`: UST の先頭が動画の何秒目か。`11.1475s`、`34b`（拍、UST のテンポで換算）、`267f`。
  `mix` では `song+34b` のように `\dviaviMark` からの時刻も書ける（前に場面を足しても音がずれない）。
- `--frames` / `--tail`（timeline）: 出力するフレーム数。`--tail 5s` は最後のキーの 5 秒後まで。どちらもなければ UST の終わりまで。
- `mix`: 音符を MML のクリップにする（`FILE:楽器[:ゲイン]`）。三連符などの半端な長さもタイで正確に表す。
  `--lyrics` を付けると UST の歌詞をモーラ（`"Ka"` など）として音符の前に書く。楽器の代わりに声の名前を指定すれば歌う。
- `script`: 歌詞の SKK キー列の雛形。1 行 1 フレーズ（1 拍以上の休符などで区切る）、1 音符 1 トークン、ひらがなで打つ形。
- `timeline`: キー列の各キーを音符の時刻に合わせ、texskk で表示するための TeX を出す。

キー列（script）の書き方:

| 書き方 | 意味 |
|---|---|
| `Si`, `suu+` など | 1 音符で打つキー。かなを完成させるキー（最後の母音、または「ん」の `n`）を発音時刻に、その前のキーは直前に、その後のキーは次の音符までの中間に打つ |
| `+` | スペースキー（変換）。`\+` は `+` そのもの |
| `^j` | C-j（確定） |
| `=` | キーを打たない音符（伸ばした母音など） |
| `!keys` | 音符を使わないキー（次の音符までの中間に打つ） |
| `#` | コメント |

timeline の出力（文書側で定義する）:

```tex
\lyricintro{フレーム数}                              % 最初のキーまで
\SKKinputto\lyricbuf{キー列}\lyricframes{フレーム数}  % 表示が変わるたび
\lyriccommit                                         % フレーズの最後のキーが打たれた（その状態の直前）
\lyricnext                                           % 次のフレーズに移る
```

## 制限

- 縦組（pTeX の `dir`）は位置と欧文の回転のみ。縦書き用の字形（句読点など）への置き換えはしない。
- 描けない描画機能は [dvipdfmx の special](#dvipdfmx-の-special) の最後を参照。
- JIS X 0213 の拡張文字（pTeX の `2004-H` の追加分）は未対応。
- AVI 1.0 なので 2 GiB まで（MS-RLE の文字主体の動画なら数時間分に相当）。
