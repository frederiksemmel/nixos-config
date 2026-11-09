# Wake word: "Stevie the TV"

`models/stevie_the_tv.tflite` is a custom openWakeWord model that
`wyoming-openwakeword` loads from `--custom-model-dir`. The app asks for it by
name (`stevie_the_tv`, the default `--wake-word`). semmeltv5 runs it with
threshold 0.9 (`hosts/semmeltv5/wake.nix`); `devenv up` runs the same model.

## The model

The model is a head on openWakeWord's shared melspectrogram and embedding
models: 16 embedding frames (1.28 s) → Dense 256 → LayerNorm → ReLU, two more
such blocks, then one logit. Its input is 18 frames: it scores the three
16-frame windows that end at the last three frames and outputs the smallest
score. A detection therefore needs three consecutive frames above the
threshold. The server's own `--trigger-level` counts frames since the last
reset rather than consecutive ones, so it can't do this.

## Measured on held-out data

All numbers are at threshold 0.9, through pyopen-wakeword exactly as the server
runs it.

| Test | Result |
| --- | --- |
| 90 clips from 6 held-out voices (en, de, es accents), quiet | 86 detected |
| the same with TV audio 10 dB below the voice | 74 detected |
| 600 held-out clips with TV 10 dB below | 80 % detected |
| 6.7 h of held-out TV (B99, Chernobyl, Silicon Valley, The Last of Us, Friends Reunion, Green Book) | 1 false detection |
| 10.7 h of openWakeWord's validation set (offline) | about 0.6 false detections per hour |

## Training

The data is synthetic and the training runs on beast's GPU in about 15 minutes.
The scripts in `train/` expect a Python venv with `torch` (CUDA),
`openwakeword` (installed with `--no-deps`), `onnxruntime`, `piper-tts`,
`scipy`, `soundfile`, `scikit-learn`, `tensorflow-cpu`, `ai-edge-litert` and
`pyopen-wakeword`. They also expect a work directory in `$WAKE_WORK`.

1. `fetch.sh` downloads:
   - openWakeWord's melspectrogram and embedding models
   - 32 Piper voices: libritts_r with 904 speakers, vctk, l2arctic,
     arctic, German and Spanish voices
   - the MIT room impulse responses

   It does not download openWakeWord's precomputed negatives
   (`openwakeword_features_ACAV100M_2000_hrs_16bit.npy`, 2,000 h of
   general audio, and `validation_set_features.npy`); put those into
   `$WAKE_WORK/data` from the openWakeWord dataset on Hugging Face.
2. TV audio: 16 kHz mono WAVs of library episodes. Training uses
   `tv/train`, about 30 h. The held-out test set in `tv/test` uses other
   episodes. `stream.py` turns them into continuous embedding frames in
   `tvfeat/`.
3. `gen.py` synthesizes clips:
   - 30k positives saying "Stevie the TV", with spelling and punctuation
     variants
   - 30k near misses ("Stevie", "Stevie Wonder", "the TV", "Steve",
     "Hey Jarvis", German and Spanish phrases, …)
   - held-out test sets from unseen speakers and voices

   It uses 12 workers, one voice per worker, so it stays under about 10 GB.
4. `features.py` builds the training windows:
   - trims each clip and puts its end 0–200 ms before the end of a 2 s
     window
   - adds room reverb (half the clips) and TV or coloured noise (85 %) at
     −3 to 20 dB SNR; the shipped model's positives used
     `--snr-min 3 --snr-max 25`
   - randomizes EQ and level
   - embeds the windows; positives go through it twice
   - also makes truncated positives (35–75 % of the phrase) as negatives
5. `train.py` trains with AdamW for 120k steps on a cosine schedule. The
   shipped model is the 100k checkpoint of `--hidden 256 --blocks 2
   --neg-weight 2000`. Each batch has:
   - 256 positives
   - 256 near misses
   - 512 TV windows
   - 1,024 ACAV windows

   Negative weights ramp up to 2,000 over the first half.
   `consec.py` compares checkpoints, thresholds and consecutive-frame
   counts on stream recall and false detections per hour.
6. `export.py CKPT OUT 3` rebuilds the head in Keras with the three-frame
   minimum, converts it to TFLite and checks it against PyTorch.
   `evalstream.py` runs the result through pyopen-wakeword on long
   recordings: `mkstream.py` makes those from clips over TV audio, and TV
   episodes count false detections.

Run every step inside `systemd-run --user --scope -p MemoryMax=40G -p
MemorySwapMax=0` on beast, which is shared.
