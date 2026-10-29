// Serves a directory (argv[2]) on a port (argv[3]): the fake release host.
const [dir, port] = [Bun.argv[2], Number(Bun.argv[3])];
Bun.serve({
  port,
  async fetch(req) {
    const file = Bun.file(`${dir}${new URL(req.url).pathname}`);
    return (await file.exists()) ? new Response(file) : new Response("not found", { status: 404 });
  },
});
