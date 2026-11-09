"""Synthesize short clips with many Piper voices: positives say the wake phrase, negatives say near misses."""
import os
import argparse, json, random, wave
from multiprocessing import Pool
from pathlib import Path
import numpy as np
from scipy.signal import resample_poly
WORK = os.environ["WAKE_WORK"]

POSITIVE = ["Stevie the TV"] * 8 + ["Stevie the TV.", "Stevie, the TV.", "Stevie the TV!", "Stevie the T.V.",
            "Stevie the tee vee", "Stevy the TV", "Steevie the TV", "Stevie the TV?"]
NEGATIVE = [
    "Stevie", "Stevie!", "Hey Stevie", "Stevie Wonder", "Stevie Nicks", "Stevie the cat", "Stevie the dog",
    "Stevie, the tea", "Stevie the teacher", "Stevie the T-Rex", "Steve", "Steve the TV guy", "Steven",
    "Stephen", "Stephanie", "Steffi", "Evie", "Stevie is here", "Stevie, come here", "Stevie, dinner is ready",
    "the TV", "turn on the TV", "turn off the TV", "on the TV", "watch TV", "see the TV", "the TV is on",
    "steady the TV", "TV", "MTV", "BBC", "CNN", "the DVD", "the TV remote", "a TV show", "it's on TV",
    "Stevie the", "see TV", "easy TV", "Steamy tea", "heavy TV", "Levi the TV", "Stevie, the movie",
    "Stevie the kitty", "every TV", "the TV series", "steve and the TV", "tell Stevie", "Stevie said",
    "very TV", "Stevie the tea pot", "Steve the tea", "she's the TV", "Stevie Ray", "Stevie's TV",
    "Hey Jarvis", "Alexa", "Hey Siri", "OK Google", "Hey Google", "Stevie Ray Vaughan", "TV time",
    "Fernseher an", "der Fernseher", "Stefan", "Steffi, mach den Fernseher an", "die Tele",
    "la tele", "Esteban", "Estefanía", "pon la tele", "Stevie y la tele",
]

def voices(directory):
    out = []
    for path in sorted(Path(directory).glob("*.onnx")):
        speakers = json.loads(Path(f"{path}.json").read_text()).get("num_speakers", 1)
        weight = 25 if "libritts_r" in path.name else 4 if speakers > 1 else 1
        out.append((path, speakers, weight))
    return out

def work(job):
    import onnxruntime
    options = onnxruntime.SessionOptions
    def single():
        o = options(); o.intra_op_num_threads = 1; o.inter_op_num_threads = 1
        return o
    onnxruntime.SessionOptions = single
    from piper import PiperVoice, SynthesisConfig
    path, speakers, job_index, n, kind, out, split, seed = job
    names = [Path(out) / f"{kind}-{seed}-{path.stem}-{job_index}-{i}.wav" for i in range(n)]
    if all(name.exists() for name in names):
        return
    rng = random.Random(f"{seed}-{path.stem}-{job_index}")
    voice = PiperVoice.load(path)
    texts = POSITIVE if kind == "pos" else NEGATIVE
    for name in names:
        speaker = None
        if speakers > 1:
            ids = range(speakers)
            if "libritts_r" in path.name:
                ids = range(850, speakers) if split == "test" else range(850)
            speaker = rng.choice(list(ids))
        config = SynthesisConfig(speaker_id=speaker, length_scale=rng.uniform(0.75, 1.35),
                                 noise_scale=rng.uniform(0.4, 1.0), noise_w_scale=rng.uniform(0.5, 1.2))
        text = rng.choice(texts)
        if name.exists():
            continue
        audio = np.concatenate([c.audio_int16_array for c in voice.synthesize(text, config)])
        audio = resample_poly(audio.astype(np.float32), 16000, voice.config.sample_rate).clip(-32768, 32767).astype(np.int16)
        with wave.open(str(name), "wb") as w:
            w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(audio.tobytes())

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("kind", choices=["pos", "neg"]); p.add_argument("count", type=int); p.add_argument("out")
    p.add_argument("--voices", default=WORK + "/voices"); p.add_argument("--split", default="train")
    p.add_argument("--seed", type=int, default=0); p.add_argument("--workers", type=int, default=12)
    a = p.parse_args()
    Path(a.out).mkdir(parents=True, exist_ok=True)
    legacy = len(list(Path(a.out).glob(f"{a.kind}-{a.seed}-[0-9]*-*.wav")))
    vlist = voices(a.voices)
    total = sum(w for _, _, w in vlist)
    jobs = []
    for path, speakers, weight in vlist:
        n = round((a.count - legacy) * weight / total)
        for j in range(0, n, 200):
            jobs.append((path, speakers, j // 200, min(200, n - j), a.kind, a.out, a.split, a.seed))
    random.Random(0).shuffle(jobs)
    with Pool(a.workers, maxtasksperchild=1) as pool:
        pool.map(work, jobs, chunksize=1)
