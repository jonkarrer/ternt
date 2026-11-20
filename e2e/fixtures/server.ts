// Serves fixture pages on :8765. Every path under /spa/ returns spa.html.
import { createServer } from "node:http";

const dir = import.meta.dir;
Bun.serve({
  port: 8765,
  async fetch(req) {
    const { pathname } = new URL(req.url);
    // A file the server says to save, not show.
    if (pathname === "/attachment")
      return new Response("attached file", {
        headers: { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="report.csv"' },
      });
    // An attachment WebKit could show (plain text): saved all the same.
    if (pathname === "/text-attachment")
      return new Response("plain notes", {
        headers: { "content-type": "text/plain", "content-disposition": 'attachment; filename="notes.txt"' },
      });
    // A form's POST, echoed: its title says what came.
    if (pathname === "/echo")
      return new Response(`<title>echo: ${req.method} ${req.method === "POST" ? await req.text() : ""}</title>`, {
        headers: { "content-type": "text/html" },
      });
    const name = pathname.startsWith("/spa") ? "spa.html" : pathname.slice(1) || "index.html";
    const file = Bun.file(`${dir}/${name}`);
    return (await file.exists()) ? new Response(file) : new Response("not found", { status: 404 });
  },
});

// :8766/slow, a file that takes a while: `chunks` pieces of 50 KB, `ms`
// apart, with its size unless `unknown` is set. (Bun.serve drops the
// Content-Length of a streamed body; node:http keeps it.)
createServer(async (req, res) => {
  const { pathname, searchParams: params } = new URL(req.url ?? "/", "http://localhost");
  if (pathname !== "/slow") return res.writeHead(404).end("not found");
  const chunks = Number(params.get("chunks") ?? 20);
  const ms = Number(params.get("ms") ?? 150);
  const piece = Buffer.alloc(50_000, 120);
  const headers: Record<string, string> = {
    "content-type": "application/octet-stream",
    "content-disposition": `attachment; filename="${params.get("name") ?? "slow.bin"}"`,
  };
  if (!params.has("unknown")) headers["content-length"] = String(chunks * piece.length);
  res.writeHead(200, headers);
  for (let i = 0; i < chunks && !res.destroyed; i++) {
    await Bun.sleep(ms);
    res.write(piece);
  }
  res.end();
}).listen(8766);
