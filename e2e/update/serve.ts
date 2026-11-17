// Serves a directory (argv[2]) on a port (argv[3]): the fake release host.
// Logs each request's path, so the test can count downloads.
const [dir, port] = [Bun.argv[2], Number(Bun.argv[3])];
Bun.serve({
  port,
  async fetch(req) {
    const path = new URL(req.url).pathname;
    console.log(path);
    const file = Bun.file(`${dir}${path}`);
    return (await file.exists()) ? new Response(file) : new Response("not found", { status: 404 });
  },
});
