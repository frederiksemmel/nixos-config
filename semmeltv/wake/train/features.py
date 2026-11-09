"""Augment clips into 2 s windows and turn them into openWakeWord embeddings (N, 16, 96)."""
import os
import argparse, random, sys
from multiprocessing import Pool
from pathlib import Path
import numpy as np, soundfile as sf
from scipy.signal import fftconvolve, butter, sosfilt
WORK = os.environ["WAKE_WORK"]

ROOT = Path(WORK)
TOTAL = 32000

def features():
    from openwakeword.utils import AudioFeatures
    return AudioFeatures(str(ROOT / "models/melspectrogram.onnx"), str(ROOT / "models/embedding_model.onnx"), ncpu=1)

def read(path):
    x, rate = sf.read(path, dtype="float32")
    assert rate == 16000, path
    return x

def trim(x):
    frames = x[: len(x) // 320 * 320].reshape(-1, 320)
    db = 10 * np.log10((frames ** 2).mean(1) + 1e-10)
    loud = np.nonzero(db > max(db.max() - 35, -60))[0]
    if len(loud) == 0:
        return x
    return x[max(0, loud[0] * 320 - 800): min(len(x), (loud[-1] + 1) * 320 + 800)]

class Backgrounds:
    def __init__(self, tv_dir):
        self.tv = [(p, sf.info(p).frames) for p in sorted(Path(tv_dir).glob("*.wav"))]
        self.rirs = [read(p) for p in sorted((ROOT / "rir/16khz").glob("*.wav"))]

    def noise(self, rng, n):
        if self.tv and rng.random() < 0.75:
            path, frames = rng.choice(self.tv)
            start = rng.randrange(0, frames - n)
            return sf.read(path, start=start, stop=start + n, dtype="float32")[0]
        white = np.random.default_rng(rng.randrange(1 << 30)).standard_normal(n).astype(np.float32)
        exponent = rng.choice([0, 1, 2])
        spectrum = np.fft.rfft(white)
        spectrum /= np.maximum(np.arange(len(spectrum)), 1) ** (exponent / 2)
        return np.fft.irfft(spectrum, n).astype(np.float32)

def rms(x):
    return np.sqrt((x ** 2).mean() + 1e-10)

SNR = (-3, 20)

def augment(x, rng, bg, truncate=False):
    x = trim(x)
    if truncate:
        x = x[: int(len(x) * rng.uniform(0.35, 0.75))]
    if rng.random() < 0.5:
        rir = rng.choice(bg.rirs)
        rir = rir[np.argmax(np.abs(rir)):]
        wet = fftconvolve(x, rir)[: len(x)]
        x = wet * (rms(x) / rms(wet))
    jitter = rng.randrange(0, 3200)
    x = x[-(TOTAL - jitter):]
    out = np.zeros(TOTAL, np.float32)
    out[TOTAL - jitter - len(x): TOTAL - jitter] = x
    speech = rms(x)
    if rng.random() < 0.85:
        noise = bg.noise(rng, TOTAL)
        snr = rng.uniform(*SNR)
        out += noise * (speech / rms(noise)) / (10 ** (snr / 20))
    if rng.random() < 0.3:
        sos = butter(2, rng.uniform(100, 400), "highpass", fs=16000, output="sos") if rng.random() < 0.5 \
            else butter(4, rng.uniform(3000, 7000), "lowpass", fs=16000, output="sos")
        out = sosfilt(sos, out).astype(np.float32)
    level = 10 ** (rng.uniform(-42, -14) / 20)
    out *= level / rms(out)
    return (out * 32767).clip(-32768, 32767).astype(np.int16)

def work(job):
    index, paths, rounds, truncate, tv_dir, seed, snr = job
    global SNR
    SNR = snr
    rng = random.Random(seed * 100003 + index)
    bg = Backgrounds(tv_dir)
    audio = np.stack([augment(read(p), rng, bg, truncate) for _ in range(rounds) for p in paths])
    return features().embed_clips(audio, batch_size=256, ncpu=1).astype(np.float16)

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("clips"); p.add_argument("out"); p.add_argument("--rounds", type=int, default=1)
    p.add_argument("--truncate", action="store_true"); p.add_argument("--tv", default=str(ROOT / "tv/train"))
    p.add_argument("--seed", type=int, default=0); p.add_argument("--workers", type=int, default=22)
    p.add_argument("--limit", type=int, default=0)
    p.add_argument("--snr-min", type=float, default=-3); p.add_argument("--snr-max", type=float, default=20)
    a = p.parse_args()
    import time
    while Path(a.out + ".lock").exists() and not Path(a.out).exists():
        time.sleep(10)
    if Path(a.out).exists():
        print(a.out, 'exists'); sys.exit(0)
    paths = sorted(Path(a.clips).glob("*.wav"))
    if a.limit:
        paths = random.Random(a.seed).sample(paths, min(a.limit, len(paths)))
    shards = [paths[i::a.workers * 4] for i in range(a.workers * 4)]
    with Pool(a.workers) as pool:
        parts = pool.map(work, [(i, s, a.rounds, a.truncate, a.tv, a.seed, (a.snr_min, a.snr_max)) for i, s in enumerate(shards) if s])
    result = np.concatenate(parts)
    np.save(a.out, result)
    print(a.out, result.shape)
