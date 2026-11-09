"""Build a long test recording: held-out clips spoken over TV audio, with the phrase end times."""
import os
import argparse, json, random
from pathlib import Path
import numpy as np, soundfile as sf
WORK = os.environ["WAKE_WORK"]

p = argparse.ArgumentParser()
p.add_argument("out"); p.add_argument("clips", nargs="+")
p.add_argument("--tv", default=WORK + "/tv/test/greenbook.wav"); p.add_argument("--snr", type=float, default=10)
p.add_argument("--seed", type=int, default=0)
a = p.parse_args()
rng = random.Random(a.seed)
paths = [q for c in a.clips for q in sorted(Path(c).glob("*.wav"))] if Path(a.clips[0]).is_dir() else [Path(c) for c in a.clips]
speech, ends, pos = [], [], 0
for path in paths:
    x, rate = sf.read(path, dtype="float32"); assert rate == 16000
    gap = np.zeros(int(16000 * rng.uniform(3, 5)), np.float32)
    speech += [gap, x]; pos += len(gap); start = pos / 16000; pos += len(x); ends.append((start, pos / 16000))
speech.append(np.zeros(16000 * 3, np.float32)); speech = np.concatenate(speech)
tv, _ = sf.read(a.tv, dtype="float32", start=16000 * 600, stop=16000 * 600 + len(speech))
voiced = np.sqrt(np.mean(np.concatenate([sf.read(q, dtype="float32")[0] for q in paths]) ** 2))
mix = speech + tv * (voiced / np.sqrt(np.mean(tv ** 2) + 1e-10)) / 10 ** (a.snr / 20) if a.snr < 99 else speech
mix *= 10 ** (-24 / 20) / np.sqrt(np.mean(mix ** 2))
sf.write(a.out, (mix * 32767).clip(-32768, 32767).astype(np.int16), 16000)
Path(a.out).with_suffix(".json").write_text(json.dumps(ends))
print(a.out, len(paths), f"{len(mix)/16000/60:.1f} min")
