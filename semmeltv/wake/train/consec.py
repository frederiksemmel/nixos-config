"""Recall on positive streams and false accepts on TV/validation for k-consecutive-frame confirmation."""
import os
import glob, json, sys
from pathlib import Path
import numpy as np, torch
sys.path.insert(0, str(Path(__file__).parent))
from train import Head, windows, score_stream
WORK = os.environ["WAKE_WORK"]
dev = "cuda"
ROOT = Path(WORK)

def detections(scores, thr, k, refractory=25):
    ok = np.lib.stride_tricks.sliding_window_view(scores, k).min(1) > thr
    hits, last = [], -10**9
    for i in np.nonzero(ok)[0]:
        j = i + k - 1
        if j - last >= refractory:
            hits.append(j); last = j
    return np.array(hits)

def t(i):
    return ((i + 15) * 1280 + 12400) / 16000

def main(ckpts):
    pos = {Path(f).stem: torch.from_numpy(np.load(f).astype(np.float32)).to(dev) for f in sorted(glob.glob(str(ROOT / "streamfeat/*.npy")))}
    neg = {"tv": [torch.from_numpy(np.load(f).astype(np.float32)).to(dev) for f in sorted(glob.glob(str(ROOT / "tvfeat/test/*.npy")))],
           "val": [torch.from_numpy(np.load(ROOT / "data/validation_set_features.npy")).to(dev)]}
    for ckpt in ckpts:
        cfg = json.loads((Path(ckpt).parent / "config.json").read_text())
        model = Head(cfg["hidden"], cfg["blocks"]).to(dev); model.load_state_dict(torch.load(ckpt)); model.eval()
        ps = {k: score_stream(model, v) for k, v in pos.items()}
        ns = {k: [score_stream(model, v) for v in vs] for k, vs in neg.items()}
        for k in [1, 2, 3, 4]:
            for thr in [0.5, 0.7, 0.8, 0.9, 0.95]:
                row = []
                for name, sc in ps.items():
                    spans = json.loads((ROOT / "streams" / f"{name}.json").read_text())
                    hits = t(detections(sc, thr, k))
                    found = sum(((hits >= s) & (hits <= e + 1.6)).any() for s, e in spans)
                    row.append(f"{name} {found / len(spans):.3f}")
                for name, scs in ns.items():
                    n = sum(len(detections(sc, thr, k)) for sc in scs)
                    h = sum(len(sc) for sc in scs) * 0.08 / 3600
                    row.append(f"{name}/h {n / h:.2f}")
                print(Path(ckpt).parent.name, Path(ckpt).stem, f"k{k} thr {thr}", " | ".join(row), flush=True)

main(sys.argv[1:])
