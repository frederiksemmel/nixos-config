"""Embed long recordings into continuous openWakeWord frames (F, 96)."""
import os
import sys
from pathlib import Path
import numpy as np, soundfile as sf
from openwakeword.utils import AudioFeatures
WORK = os.environ["WAKE_WORK"]

f = AudioFeatures(WORK + "/models/melspectrogram.onnx", WORK + "/models/embedding_model.onnx", ncpu=8)
for wav in sys.argv[2:]:
    out = Path(sys.argv[1]) / (Path(wav).stem + ".npy")
    if out.exists():
        continue
    x, rate = sf.read(wav, dtype="int16")
    chunk, frames = 1280 * 4000, []
    for start in range(0, len(x), chunk):
        piece = x[start: start + chunk + 12160 + 640]
        if len(piece) < 16000:
            break
        emb = f._get_embeddings(piece)
        frames.append(emb[: chunk // 1280])
    np.save(out, np.concatenate(frames).astype(np.float16))
    print(out, sum(len(e) for e in frames))
