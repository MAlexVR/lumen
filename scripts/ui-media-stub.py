#!/usr/bin/env python3
"""Creates placeholder videos for every lesson in the UI fixture.

    python3 scripts/ui-media-stub.py OUT_DIR

Writes one tiny generated clip (ffmpeg test pattern) and copies it to every
`video` path in ui/src/lib/mock/library.json under OUT_DIR, so
`scripts/ui-shot.py --media-root OUT_DIR` can play something without any real
library. Any folder that follows the same `Platform/Group/Course/...` layout
works as a media root too.
"""
import json
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def video_paths(fixture):
    for platform in fixture["platforms"]:
        for group in platform["groups"]:
            for course in group["courses"]:
                for section in course["sections"]:
                    for lesson in section["lessons"]:
                        paths = [lesson["video"]] if lesson["video"] else []
                        paths += [r["path"] for r in lesson["resources"] if r["kind"] == "video"]
                        yield from paths


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    clip = os.path.join(out, ".clip.mp4")
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=20",
         "-f", "lavfi", "-i", "sine=frequency=220:duration=20", "-c:v", "libx264", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-shortest", clip],
        check=True,
    )
    with open(os.path.join(ROOT, "ui/src/lib/mock/library.json"), encoding="utf-8") as f:
        fixture = json.load(f)
    count = 0
    for rel in video_paths(fixture):
        dest = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copyfile(clip, dest)
        count += 1
    os.remove(clip)
    print(f"{count} placeholder videos under {out}")


if __name__ == "__main__":
    main()
