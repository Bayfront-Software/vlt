"""README の先頭に置く端末の動画（docs/demo.gif）を作る。

画面の出力は vlt 0.3.1 を一時 vault で実際に動かして得たもの（2026-09-30）。
vlt の出力の形を変えたら、実際に動かし直して下の SCRIPT を合わせること。

    python3 scripts/make_demo_gif.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parent.parent / "docs" / "demo.gif"
FONT = ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", 15)
WIDTH, HEIGHT = 900, 340
PAD_X, TOP, LINE_H = 22, 52, 24
BG, BAR = (24, 26, 33), (38, 41, 51)
FG, DIM, PROMPT, ACCENT = (222, 226, 234), (122, 130, 148), (126, 211, 133), (242, 183, 76)

# (種類, 文字列)。cmd は1文字ずつ打つ、out は一度に出す、note は手順の説明。
SCRIPT = [
    ("note", "# 1. keep the secret in the local vault (not in your shell history)"),
    ("cmd", "pbpaste | vlt set openai/api-key"),
    ("out", "Secret stored: openai/api-key"),
    ("blank", ""),
    ("note", "# 2. .env holds only a reference - safe to commit"),
    ("cmd", "cat .env"),
    ("out", "OPENAI_API_KEY=vlt://openai/api-key"),
    ("blank", ""),
    ("note", "# 3. inject at run time; piped output (logs, CI, AI agents) is masked"),
    ("cmd", "vlt run --env-file .env -- sh -c 'echo \"Using key: $OPENAI_API_KEY\"' | tee run.log"),
    ("out", "Using key: <concealed by vlt>"),
]
MASK = "<concealed by vlt>"
CHARS_PER_FRAME = 3


def draw_line(d: ImageDraw.ImageDraw, y: int, kind: str, text: str) -> None:
    x = PAD_X
    if kind == "cmd":
        d.text((x, y), "$ ", font=FONT, fill=PROMPT)
        d.text((x + FONT.getlength("$ "), y), text, font=FONT, fill=FG)
    elif kind == "blank":
        return
    elif kind == "note":
        d.text((x, y), text, font=FONT, fill=DIM)
    elif MASK in text:
        before = text.split(MASK)[0]
        d.text((x, y), before, font=FONT, fill=FG)
        d.text((x + FONT.getlength(before), y), MASK, font=FONT, fill=ACCENT)
    else:
        d.text((x, y), text, font=FONT, fill=FG)


def frame(lines: list[tuple[str, str]], cursor: bool) -> Image.Image:
    img = Image.new("RGB", (WIDTH, HEIGHT), BG)
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, WIDTH, 34), fill=BAR)
    for i, color in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        d.ellipse((16 + i * 20, 11, 28 + i * 20, 23), fill=color)
    title = "vlt - local secret manager"
    d.text(((WIDTH - FONT.getlength(title)) / 2, 8), title, font=FONT, fill=DIM)
    y = TOP
    for kind, text in lines:
        draw_line(d, y, kind, text)
        y += LINE_H
    if cursor and lines:
        kind, text = lines[-1]
        cx = PAD_X + FONT.getlength(("$ " if kind == "cmd" else "") + text)
        d.rectangle((cx + 1, y - LINE_H + 3, cx + 9, y - 4), fill=FG)
    return img


def main() -> None:
    frames: list[Image.Image] = []
    durations: list[int] = []
    shown: list[tuple[str, str]] = []

    def add(lines: list[tuple[str, str]], ms: int, cursor: bool = False) -> None:
        frames.append(frame(lines, cursor))
        durations.append(ms)

    add([], 600)
    for kind, text in SCRIPT:
        if kind == "cmd":
            for n in range(0, len(text) + 1, CHARS_PER_FRAME):
                add(shown + [(kind, text[:n])], 45, cursor=True)
            shown.append((kind, text))
            add(shown, 500, cursor=True)
        else:
            shown.append((kind, text))
            add(shown, {"out": 900, "note": 350, "blank": 0}[kind]) if kind != "blank" else None
    add(shown, 3500)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    # 色数を削りすぎると信号の3色がくすむので、最後のコマから作った共通パレットで全コマをそろえる
    base = frames[-1].quantize(colors=64, method=Image.Quantize.MEDIANCUT)
    palette = [f.quantize(palette=base, dither=Image.Dither.NONE) for f in frames]
    palette[0].save(OUT, save_all=True, append_images=palette[1:], duration=durations, loop=0, optimize=True)
    print(f"{OUT} ({OUT.stat().st_size // 1024} KB, {len(frames)} frames, {sum(durations) / 1000:.1f}s)")


if __name__ == "__main__":
    main()
