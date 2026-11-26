// The matrix-sdk engine's worker (src/engine/worker.rs). The page posts its
// own compiled wasm module and the URL of its wasm-bindgen glue; this loads
// them here and starts the engine, which then takes over the messages.
self.onmessage = async ({data}) => {
    self.onmessage = null;
    try {
        const glue = await import(data.glue);
        await glue.default({module_or_path: data.module});
        glue.untethered_engine_worker();
    } catch (error) {
        // A category only: exception text can carry URLs and server data.
        self.postMessage({json: JSON.stringify({Stopped: "The matrix-sdk engine couldn’t start in a browser worker. Reload this tab to retry."})});
        throw error;
    }
};
