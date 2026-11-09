"""Rebuild a trained head in Keras, convert it to TFLite and check it against PyTorch."""
import sys, json
from pathlib import Path
import numpy as np, torch
sys.path.insert(0, str(Path(__file__).parent))
from train import Head
import tensorflow as tf

ckpt, out, k = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
config = json.loads((ckpt.parent / "config.json").read_text())
head = Head(config["hidden"], config["blocks"]); head.load_state_dict(torch.load(ckpt, map_location="cpu")); head.eval()
layers = [l for l in head.net if not isinstance(l, (torch.nn.Flatten, torch.nn.ReLU))]
keras_layers = [(tf.keras.layers.Dense(l.out_features) if isinstance(l, torch.nn.Linear) else tf.keras.layers.LayerNormalization(epsilon=l.eps), l) for l in layers]
def head_keras(x):
    x = tf.keras.layers.Flatten()(x)
    for kl, l in keras_layers:
        x = kl(x)
        if not isinstance(l, torch.nn.Linear):
            x = tf.keras.layers.ReLU()(x)
    return x
inp = tf.keras.Input(shape=(15 + k, 96), batch_size=1)
scores = [head_keras(inp[:, i:i + 16, :]) for i in range(k)]
x = tf.keras.ops.min(tf.keras.layers.Concatenate(axis=1)(scores), axis=1, keepdims=True) if k > 1 else scores[0]
x = tf.keras.layers.Activation("sigmoid")(x)
model = tf.keras.Model(inp, x)
for kl, l in keras_layers:
    kl.set_weights([l.weight.detach().numpy().T, l.bias.detach().numpy()] if isinstance(l, torch.nn.Linear) else [l.weight.detach().numpy(), l.bias.detach().numpy()])
converter = tf.lite.TFLiteConverter.from_keras_model(model)
out.write_bytes(converter.convert())
from ai_edge_litert.interpreter import Interpreter
it = Interpreter(model_path=str(out)); it.allocate_tensors()
i, o = it.get_input_details()[0], it.get_output_details()[0]
print("input", i["shape"], "output", o["shape"])
rng = np.random.default_rng(0); worst = 0
for _ in range(50):
    x = (rng.standard_normal((1, 15 + k, 96)) + 1.9).astype(np.float32)
    it.set_tensor(i["index"], x); it.invoke()
    a = it.get_tensor(o["index"]).item()
    b = min(torch.sigmoid(head(torch.from_numpy(x[:, j:j + 16]))).item() for j in range(k))
    worst = max(worst, abs(a - b))
print("max |tflite - torch|", worst, out, out.stat().st_size)
