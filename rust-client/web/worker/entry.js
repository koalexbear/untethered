// Let Vite resolve and package the pinned upstream worker and both WASM assets.
import GomuksWorker from "./wasmuks.ts?worker";
export const createWorker = () => new GomuksWorker();
