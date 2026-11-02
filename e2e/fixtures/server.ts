// Serves fixture pages on :8765. Every path under /spa/ returns spa.html.
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
    const name = pathname.startsWith("/spa") ? "spa.html" : pathname.slice(1) || "index.html";
    const file = Bun.file(`${dir}/${name}`);
    return (await file.exists()) ? new Response(file) : new Response("not found", { status: 404 });
  },
});
