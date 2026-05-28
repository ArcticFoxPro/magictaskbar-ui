import fs from "fs";
import path from "path";
import { Buffer } from "node:buffer";

function patchFile(filePath: string) {
  if (!fs.existsSync(filePath)) return false;
  const buf = fs.readFileSync(filePath);
  const targets = [
    Buffer.from("armony", "ascii"),
  ];
  const replacements = [
    Buffer.from("arm0ny", "ascii"),
  ];

  let modified = false;
  for (let t = 0; t < targets.length; t++) {
    const needle: Buffer = targets[t]!;
    const repl: Buffer = replacements[t]!;
    let idx = 0;
    while (idx <= buf.length - needle.length) {
      let match = true;
      for (let j = 0; j < needle.length; j++) {
        if (buf[idx + j] !== needle[j]!) {
          match = false;
          break;
        }
      }
      if (match) {
        // Replace in-place with same-length bytes
        for (let j = 0; j < needle.length; j++) {
          buf[idx + j] = repl[j]! as number;
        }
        modified = true;
        idx += needle.length;
      } else {
        idx++;
      }
    }
  }

  if (modified) {
    const bak = filePath + ".bak";
    try {
      if (!fs.existsSync(bak)) fs.copyFileSync(filePath, bak);
    } catch {}
    fs.writeFileSync(filePath, buf);
  }
  return modified;
}

const roots = [
  path.join("target", "debug", "magictaskbar-ui.exe"),
  path.join("target", "release", "magictaskbar-ui.exe"),
];

let any = false;
for (const p of roots) {
  const ok = patchFile(p);
  if (ok) {
    console.info(`Patched: ${p}`);
    any = true;
  }
}

if (!any) console.info("No EXE patched (files not found or no occurrences)");
