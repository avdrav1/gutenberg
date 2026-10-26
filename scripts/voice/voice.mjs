// The tour's voiceover: each line of docs/VOICEOVER.md spoken by Kokoro-82M,
// an Apache-2.0 model run on this machine (kokoro-js), and laid on one track
// at the moment its caption comes up. scripts/site-media.sh runs this and
// mixes the track over the music.
//
//   node voice.mjs <VOICEOVER.md> <cues> <seconds> <out.wav>
//
// <cues> has a line per scene, `<seconds>\t<caption>`, the title and install
// cards as "Title card" and "Install card". Clips are cached in .voice/ by
// voice, speed and text, so only a changed line is spoken again. The model
// downloads once from Hugging Face, then works offline.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";

const VOICE = process.env.GUTENBERG_VOICE ?? "af_heart";
const SPEED = Number(process.env.GUTENBERG_VOICE_SPEED ?? 0.95);
// The scene's first frame lands before the voice does.
const LEAD = 0.3;
// The least room between two lines.
const GAP = 0.2;

const [script, cuesFile, seconds, out] = process.argv.slice(2);
if (!out) {
  console.error("usage: voice.mjs <VOICEOVER.md> <cues> <seconds> <out.wav>");
  process.exit(2);
}
const total = Number(seconds);

// The script's table: `| Time | Caption | Voiceover |`.
const lines = (await readFile(script, "utf8"))
  .split("\n")
  .filter((l) => l.startsWith("| ") && !l.startsWith("| Time"))
  .map((l) => l.split("|").map((c) => c.trim()))
  .map(([, , caption, text]) => ({ caption, text }));
const cues = new Map(
  (await readFile(cuesFile, "utf8"))
    .split("\n")
    .filter(Boolean)
    .map((l) => l.split("\t"))
    .map(([at, caption]) => [caption, Number(at)]),
);
// A line may run over scenes with no line of their own, up to the next that has one.
const starts = lines
  .map((l) => cues.get(l.caption))
  .filter((s) => s !== undefined)
  .sort((a, b) => a - b);

const cache = join(import.meta.dirname, ".voice");
await mkdir(cache, { recursive: true });
let tts;
const RATE = 24000;
const track = new Float32Array(Math.ceil(total * RATE));
let free = 0;
let ok = true;
for (const { caption, text } of lines) {
  const cue = cues.get(caption);
  if (cue === undefined) {
    console.error(`voiceover: no scene has the caption "${caption}"`);
    ok = false;
    continue;
  }
  const key = createHash("sha256").update(`${VOICE}\n${SPEED}\n${text}`).digest("hex").slice(0, 16);
  const file = join(cache, `${key}.f32`);
  let clip;
  if (existsSync(file)) {
    const buf = await readFile(file);
    clip = new Float32Array(buf.buffer, buf.byteOffset, buf.length / 4);
  } else {
    if (!tts) {
      const { KokoroTTS } = await import("kokoro-js");
      tts = await KokoroTTS.from_pretrained("onnx-community/Kokoro-82M-v1.0-ONNX", { dtype: "fp32", device: "cpu" });
    }
    const audio = await tts.generate(text, { voice: VOICE, speed: SPEED });
    if (audio.sampling_rate !== RATE) throw new Error(`Kokoro spoke at ${audio.sampling_rate} Hz`);
    clip = trim(audio.audio);
    await writeFile(file, Buffer.from(clip.buffer, clip.byteOffset, clip.byteLength));
  }
  let at = cue + LEAD;
  if (at < free + GAP) {
    console.error(`voiceover: "${caption}" waits ${(free + GAP - at).toFixed(1)} s for the line before it`);
    at = free + GAP;
  }
  const end = at + clip.length / RATE;
  const next = starts.find((s) => s > cue) ?? total;
  if (end > next + 0.5) {
    console.error(`voiceover: "${caption}" runs ${(end - next).toFixed(1)} s into the next line's scene; shorten it`);
  }
  if (end > total) {
    console.error(`voiceover: "${caption}" runs past the end`);
    ok = false;
    continue;
  }
  track.set(clip, Math.round(at * RATE));
  free = end;
  console.log(`  ${at.toFixed(1).padStart(5)} s  ${(clip.length / RATE).toFixed(1)} s  ${caption}`);
}
await writeFile(out, wav(track));
process.exit(ok ? 0 : 1);

// Cut the silence the speaker leaves at either end, keeping 40 ms.
function trim(x) {
  const loud = (v) => Math.abs(v) > 0.01;
  let a = 0, b = x.length;
  while (a < b && !loud(x[a])) a++;
  while (b > a && !loud(x[b - 1])) b--;
  const pad = Math.round(0.04 * RATE);
  return x.slice(Math.max(0, a - pad), Math.min(x.length, b + pad));
}

function wav(x) {
  const out = Buffer.alloc(44 + x.length * 2);
  out.write("RIFF", 0); out.writeUInt32LE(36 + x.length * 2, 4); out.write("WAVE", 8);
  out.write("fmt ", 12); out.writeUInt32LE(16, 16); out.writeUInt16LE(1, 20); out.writeUInt16LE(1, 22);
  out.writeUInt32LE(RATE, 24); out.writeUInt32LE(RATE * 2, 28); out.writeUInt16LE(2, 32); out.writeUInt16LE(16, 34);
  out.write("data", 36); out.writeUInt32LE(x.length * 2, 40);
  for (let i = 0; i < x.length; i++) out.writeInt16LE(Math.round(Math.max(-1, Math.min(1, x[i])) * 32767), 44 + i * 2);
  return out;
}
