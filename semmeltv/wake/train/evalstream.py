"""Run WAVs through pyopen-wakeword exactly like wyoming-openwakeword and report detections."""
import argparse, sys, time
from pathlib import Path
import numpy as np, soundfile as sf
from pyopen_wakeword import OpenWakeWord, OpenWakeWordFeatures

def run(model_path, audio, threshold, refractory=2.0, chunk=1024):
    features = OpenWakeWordFeatures.from_builtin()
    model = OpenWakeWord.from_model(model_path)
    hits, last, peak = [], None, []
    pos = 0
    for start in range(0, len(audio), chunk):
        piece = audio[start:start + chunk]
        pos = start + len(piece)
        for emb in features.process_streaming(piece):
            for prob in model.process_streaming(emb):
                t = pos / 16000
                peak.append((t, prob))
                if prob > threshold and (last is None or t - last >= refractory):
                    hits.append(t); last = t
    return hits, peak

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("model"); p.add_argument("wavs", nargs="+"); p.add_argument("--threshold", type=float, default=0.5)
    a = p.parse_args()
    total_h, total_hits = 0, 0
    for wav in a.wavs:
        x, rate = sf.read(wav, dtype="int16"); assert rate == 16000
        hits, _ = run(a.model, x, a.threshold)
        hours = len(x) / 16000 / 3600
        total_h += hours; total_hits += len(hits)
        truth = Path(wav).with_suffix(".json")
        if truth.exists():
            import json
            ends = json.loads(truth.read_text())
            found = sum(any(s <= h <= e + 1.6 for h in hits) for s, e in ends)
            extra = sum(not any(s <= h <= e + 1.6 for s, e in ends) for h in hits)
            print(f"{Path(wav).name}: recall {found}/{len(ends)} = {found/len(ends):.3f}, {extra} false", flush=True)
            continue
        print(f"{Path(wav).name}: {len(hits)} detections in {hours:.2f} h at", " ".join(f"{h/60:.1f}m" for h in hits[:20]), flush=True)
    print(f"total {total_hits} in {total_h:.2f} h = {total_hits/total_h:.2f}/h")
