// Regenerates bench/ts_outputs.json from a canonic checkout (default ../../canonic):
//   node bench/dump.mjs [path to canonic]
// The payload test compares person.py and routing.py against these bodies byte for byte.
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(process.argv[2] ?? fileURLToPath(new URL("../../canonic", import.meta.url)));
const load = (p) => import(pathToFileURL(resolve(root, p)).href);
const { compileSpec } = await load("packages/canonic/dist/index.js");
const person = await load("bench/person.js");
const routing = await load("bench/routing.js");

const payloads = Array.from({ length: 64 }, (_, i) => person.payload(i));
const raws = payloads.map((p) => JSON.stringify(p));
const art = compileSpec(person.spec, person.canonical);
const anc = compileSpec(person.anchoredSpec, person.canonical);
const big = Array.from({ length: 64 }, (_, i) => routing.payload(i));

writeFileSync(
  new URL("ts_outputs.json", import.meta.url),
  JSON.stringify({
    ts_commit: execFileSync("git", ["-C", root, "rev-parse", "--short", "HEAD"], { encoding: "utf8" }).trim(),
    raws,
    expected: payloads.map((p) => art.map(p)),
    routing: raws.map((r) => art.extract(r)),
    anchored: raws.map((r) => anc.extract(r)),
    routing_raws: big.map((p) => JSON.stringify(p)),
    hand_read: big.map((p) => routing.handRead(p)),
  }) + "\n",
);
