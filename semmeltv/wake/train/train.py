"""Train the Stevie the TV head on openWakeWord embeddings and keep checkpoints with their metrics."""
import os
import argparse, glob, json, math, time
from pathlib import Path
import numpy as np, torch, torch.nn as nn
WORK = os.environ["WAKE_WORK"]

ROOT = Path(WORK)
dev = "cuda"

def load(pattern):
    files = sorted(glob.glob(str(ROOT / pattern)))
    return np.concatenate([np.load(f) for f in files]) if files else np.zeros((0, 16, 96), np.float16)

def streams(pattern):
    return [torch.from_numpy(np.load(f).astype(np.float32)) for f in sorted(glob.glob(str(ROOT / pattern)))]

class Head(nn.Module):
    def __init__(self, hidden, blocks):
        super().__init__()
        layers = [nn.Flatten(), nn.Linear(16 * 96, hidden), nn.LayerNorm(hidden), nn.ReLU()]
        for _ in range(blocks):
            layers += [nn.Linear(hidden, hidden), nn.LayerNorm(hidden), nn.ReLU()]
        layers += [nn.Linear(hidden, 1)]
        self.net = nn.Sequential(*layers)

    def forward(self, x):
        return self.net(x).squeeze(-1)

def windows(stream, idx):
    offsets = torch.arange(16, device=idx.device)
    return stream[idx[:, None] + offsets[None, :]]

@torch.no_grad()
def score_stream(model, stream):
    out = []
    for start in range(0, len(stream) - 15, 65536):
        idx = torch.arange(start, min(start + 65536, len(stream) - 15), device=dev)
        out.append(torch.sigmoid(model(windows(stream, idx))))
    return torch.cat(out).cpu().numpy()

def events(scores, threshold, refractory=25):
    count, last = 0, -10**9
    for i in np.nonzero(scores > threshold)[0]:
        if i - last >= refractory:
            count += 1; last = i
        # the server rearms only after the refractory time since the last trigger
    return count

@torch.no_grad()
def score(model, x):
    return torch.cat([torch.sigmoid(model(x[i:i + 65536])) for i in range(0, len(x), 65536)]).cpu().numpy()

def main():
    p = argparse.ArgumentParser()
    p.add_argument("name"); p.add_argument("--steps", type=int, default=40000)
    p.add_argument("--hidden", type=int, default=64); p.add_argument("--blocks", type=int, default=1)
    p.add_argument("--pos", default="feat/pos-train*.npy"); p.add_argument("--hard", default="")
    p.add_argument("--neg-weight", type=float, default=30.0); p.add_argument("--lr", type=float, default=5e-4)
    a = p.parse_args()
    torch.manual_seed(0)
    out = ROOT / "runs" / a.name; out.mkdir(parents=True, exist_ok=True)

    pos = torch.from_numpy(load(a.pos).astype(np.float32)).to(dev)
    adv = torch.from_numpy(load("feat/neg-train*.npy").astype(np.float32)).to(dev)
    hard = torch.from_numpy(load(a.hard).astype(np.float32)).to(dev) if a.hard else None
    tv = [s.to(dev) for s in streams("tvfeat/train/*.npy")]
    acav = np.load(ROOT / "data/openwakeword_features_ACAV100M_2000_hrs_16bit.npy", mmap_mode="r")
    val_pos = {k: torch.from_numpy(load(f"feat/{k}*.npy").astype(np.float32)).to(dev) for k in ["pos-test", "pos-heldout", "quiet-heldout", "pos-train"]}
    val_pos["pos-train"] = val_pos["pos-train"][:5000]
    val_pos = {k: v for k, v in val_pos.items() if len(v)}
    val_adv = torch.from_numpy(load("feat/neg-test*.npy").astype(np.float32)).to(dev)
    val_streams = {"validation": torch.from_numpy(np.load(ROOT / "data/validation_set_features.npy")).to(dev)}
    for f in sorted(glob.glob(str(ROOT / "tvfeat/test/*.npy"))):
        val_streams["tv:" + Path(f).stem] = torch.from_numpy(np.load(f).astype(np.float32)).to(dev)
    print("pos", pos.shape, "adv", adv.shape, "tv", sum(len(s) for s in tv), "val", {k: v.shape for k, v in val_pos.items()}, flush=True)

    model = Head(a.hidden, a.blocks).to(dev)
    opt = torch.optim.AdamW(model.parameters(), lr=a.lr, weight_decay=1e-2)
    warm = 1000
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: min(1, (s + 1) / warm) * 0.5 * (1 + math.cos(math.pi * min(s, a.steps) / a.steps)))
    bce = nn.BCEWithLogitsLoss(reduction="none")
    rng = np.random.default_rng(0)
    # ACAV is too large for the GPU, so stream contiguous blocks of it from the memory map
    block, block_rows = None, 1 << 18
    history = []
    t0 = time.time()
    for step in range(a.steps + 1):
        if step % 500 == 0:
            start = rng.integers(0, len(acav) - block_rows)
            block = torch.from_numpy(np.asarray(acav[start:start + block_rows], dtype=np.float32)).to(dev)
        s = [pos[torch.randint(len(pos), (256,), device=dev)],
             adv[torch.randint(len(adv), (256,), device=dev)] if len(adv) else block[torch.randint(block_rows, (256,), device=dev)]]
        stream = tv[step % len(tv)]
        s.append(windows(stream, torch.randint(len(stream) - 15, (512,), device=dev)))
        s.append(block[torch.randint(block_rows, (1024,), device=dev)])
        if hard is not None:
            s.append(hard[torch.randint(len(hard), (512,), device=dev)])
        x = torch.cat(s)
        y = torch.cat([torch.ones(256, device=dev), torch.zeros(len(x) - 256, device=dev)])
        ramp = 1 + (a.neg_weight - 1) * min(1.0, step / (0.5 * a.steps))
        w = torch.where(y > 0, torch.ones_like(y), torch.full_like(y, ramp))
        x = x + 0.02 * torch.randn_like(x) * (y[:, None, None] > 0)
        loss = (bce(model(x), y) * w).sum() / w.sum()
        opt.zero_grad(); loss.backward(); opt.step(); sched.step()
        if step % 10000 == 0 and step >= 20000 or step == a.steps:
            model.eval()
            metrics = {"step": step, "loss": float(loss)}
            for thr in [0.5, 0.7, 0.8, 0.9, 0.95]:
                m = {k: float((score(model, v) > thr).mean()) for k, v in val_pos.items()}
                if len(val_adv):
                    m["adv"] = float((score(model, val_adv) > thr).mean())
                for k, v in val_streams.items():
                    sc = score_stream(model, v)
                    m[k + "/h"] = events(sc, thr) / (len(v) * 0.08 / 3600)
                metrics[str(thr)] = m
            history.append(metrics)
            torch.save(model.state_dict(), out / f"step{step}.pt")
            print(json.dumps(metrics), f"{time.time() - t0:.0f}s", flush=True)
            model.train()
    (out / "history.json").write_text(json.dumps(history, indent=1))
    (out / "config.json").write_text(json.dumps(vars(a)))

if __name__ == "__main__":
    main()
