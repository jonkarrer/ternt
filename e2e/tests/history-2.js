// History persists across restarts (history-1 left one visit).
await t.until(async () => (await t.invoke("history_search", { query: "kept", limit: 10 })).length === 1, "history restored from disk");
await t.quit();
