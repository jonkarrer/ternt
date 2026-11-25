// Writes src-tauri/third-party-licenses.txt: the license of every package that ends up in
// a release (Rust crates linked into Ternt, and the npm packages
// bundled into the pages), with the license texts the packages carry. Their licenses ask
// for these notices to travel with the app; Help → Third-Party Licenses shows the file.
//
//   bun scripts/licenses.ts          (or `just licenses`), after dependencies change
//   bun scripts/licenses.ts --check  fails if the file is out of date

import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const root = join(import.meta.dir, "..");
const OUT = join(root, "src-tauri/third-party-licenses.txt");
const TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
const TEXT_FILE = /^(licen[sc]e|copying|notice|unlicense)/i;

type Pkg = { name: string; version: string; license: string; home: string; texts: string[] };
const found = new Map<string, Pkg>();

/** The license files in a package's folder, tidied so the same text compares equal. */
function texts(dir: string): string[] {
  return readdirSync(dir)
    .filter((file) => TEXT_FILE.test(file))
    .sort()
    .map((file) => {
      try {
        return readFileSync(join(dir, file), "utf8")
          .replace(/\r\n?/g, "\n")
          .split("\n")
          .map((line) => line.trimEnd())
          .join("\n")
          .trim();
      } catch {
        return ""; // a folder named LICENSES, say
      }
    })
    .filter((text) => text !== "");
}

/** Crates a workspace's shipped package links: normal dependencies, for the Macs we build for. */
function crates(manifest: string, roots: string[]) {
  for (const target of TARGETS) {
    const run = Bun.spawnSync(
      ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target, "--manifest-path", manifest],
      { stderr: "inherit" },
    );
    if (run.exitCode !== 0) throw new Error(`cargo metadata failed for ${manifest}`);
    const meta = JSON.parse(run.stdout.toString());
    const packages = new Map<string, any>(meta.packages.map((p: any) => [p.id, p]));
    const nodes = new Map<string, any>(meta.resolve.nodes.map((n: any) => [n.id, n]));
    const todo = meta.packages.filter((p: any) => roots.includes(p.name) && p.source === null).map((p: any) => p.id);
    if (todo.length !== roots.length) throw new Error(`${manifest}: expected the packages ${roots.join(", ")}`);
    const seen = new Set<string>(todo);
    while (todo.length > 0) {
      const id = todo.pop()!;
      const p = packages.get(id);
      // Our own crates (no registry source) are under Ternt's license, not listed here.
      if (p.source !== null) {
        found.set(`crate ${p.name} ${p.version}`, {
          name: p.name,
          version: p.version,
          license: p.license ?? "see its license file",
          home: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
          texts: texts(dirname(p.manifest_path)),
        });
      }
      for (const dep of nodes.get(id).deps) {
        if (!dep.dep_kinds.some((k: any) => k.kind === null) || seen.has(dep.pkg)) continue;
        seen.add(dep.pkg);
        todo.push(dep.pkg);
      }
    }
  }
}

/** npm packages the pages bundle: package.json's dependencies, Svelte, and what they depend on. */
function npm() {
  // Svelte is a devDependency (a compiler), but its runtime is in every page.
  const todo = [...Object.keys(JSON.parse(readFileSync(join(root, "package.json"), "utf8")).dependencies ?? {}), "svelte"];
  const seen = new Set(todo);
  while (todo.length > 0) {
    const name = todo.pop()!;
    const dir = join(root, "node_modules", name);
    if (!existsSync(join(dir, "package.json"))) throw new Error(`${name} isn't installed: run bun install`);
    const p = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const repo = typeof p.repository === "string" ? p.repository : p.repository?.url;
    found.set(`npm ${p.name} ${p.version}`, {
      name: p.name,
      version: p.version,
      license: typeof p.license === "string" ? p.license : "see its license file",
      home: p.homepage ?? repo ?? `https://www.npmjs.com/package/${p.name}`,
      texts: texts(dir),
    });
    for (const dep of Object.keys(p.dependencies ?? {})) {
      if (seen.has(dep)) continue;
      seen.add(dep);
      todo.push(dep);
    }
  }
}

crates(join(root, "src-tauri/Cargo.toml"), ["ternt"]);
npm();

const all = [...found.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
const rule = "=".repeat(78);
const lines = [
  "Third-party software in Ternt",
  "",
  "Ternt includes the software listed here. Each package belongs to its authors and is",
  "used under the license named beside it. Part 1 lists the packages; part 2 gives the",
  "license texts they carry, each followed by the packages it came with.",
  "",
  `${all.length} packages. Written by scripts/licenses.ts; do not edit by hand.`,
  "",
  rule,
  "Part 1: packages",
  rule,
  "",
  ...all.map((p) => `${p.name} ${p.version}  [${p.license}]  ${p.home}`),
];

// The same text (Apache-2.0, mostly) is printed once, with everything that carries it.
const byText = new Map<string, string[]>();
for (const p of all) {
  for (const text of p.texts) byText.set(text, [...(byText.get(text) ?? []), `${p.name} ${p.version}`]);
}
lines.push("", rule, "Part 2: license texts", rule);
for (const [text, users] of [...byText].sort((a, b) => b[1].length - a[1].length || a[1][0].localeCompare(b[1][0]))) {
  lines.push("", `Used by: ${users.join(", ")}`, "-".repeat(78), "", text, "", rule);
}
const without = all.filter((p) => p.texts.length === 0);
if (without.length > 0) {
  lines.push("", "These packages ship no license file; their license is the one named in part 1:", "");
  lines.push(...without.map((p) => `${p.name} ${p.version}  [${p.license}]  ${p.home}`));
}
const output = lines.join("\n") + "\n";

if (process.argv.includes("--check")) {
  if (!existsSync(OUT) || readFileSync(OUT, "utf8") !== output) {
    console.error("src-tauri/third-party-licenses.txt is out of date: run `just licenses`");
    process.exit(1);
  }
} else {
  writeFileSync(OUT, output);
  console.log(`${all.length} packages, ${byText.size} license texts, ${without.length} without a file -> ${OUT}`);
}
