export default {
  base: "./",
  worker: {
    format: "es",
    rollupOptions: { output: {
      entryFileNames: chunk => chunk.name === "wasmuks" ? "gomuks-worker.js" : "assets/[name]-[hash].js",
    } },
  },
  build: {
    target: "esnext",
    rollupOptions: { input: "entry.js", preserveEntrySignatures: "strict" },
  },
};
