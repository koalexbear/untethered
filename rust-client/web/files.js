let stage;
export function install_files(onFiles, hover) {
    stage = onFiles;
    const hasFiles = e => e.dataTransfer?.types.includes("Files");
    window.addEventListener("dragover", e => {
        if (!hasFiles(e)) return;
        e.preventDefault(); e.dataTransfer.dropEffect = "copy";
        if (!document.documentElement.hasAttribute("data-file-drag")) {
            document.documentElement.setAttribute("data-file-drag", ""); hover(true);
        }
    });
    const end = () => { document.documentElement.removeAttribute("data-file-drag"); hover(false); };
    window.addEventListener("dragleave", e => { if (!e.relatedTarget) end(); });
    window.addEventListener("drop", e => {
        if (!hasFiles(e)) return;
        e.preventDefault(); end(); onFiles("", e.dataTransfer.files);
    });
    window.addEventListener("paste", e => {
        if (e.target instanceof HTMLInputElement || !e.clipboardData?.files.length) return;
        e.preventDefault(); onFiles("", e.clipboardData.files);
    });
}
export function pick(room) {
    const input = document.createElement("input");
    input.type = "file"; input.multiple = true;
    input.onchange = () => { if (input.files.length) stage(room, input.files); };
    input.click();
}
const reading = new Map();
export async function post_upload(worker, id, file, encrypt) {
    const reader = new FileReader();
    reading.set(id,reader);
    try {
        await new Promise((resolve,reject)=>{
            reader.onload=resolve;
            reader.onerror=reader.onabort=()=>reject(new Error("Upload preparation stopped"));
            reader.readAsArrayBuffer(file);
        });
        if (reading.get(id)!==reader) throw new Error("Upload cancelled");
        const buffer=reader.result;
        worker.postMessage({command:"wasm-upload",request_id:id,data:"",encrypt,filename:file.name,payload:new Uint8Array(buffer)},[buffer]);
    } finally {reading.delete(id);}
}
export function cancel_upload(worker,id){
    const reader=reading.get(id);
    reading.delete(id);
    if (reader?.readyState===FileReader.LOADING) reader.abort();
    worker.postMessage({command:"wasm-upload-cancel",request_id:id,data:"{}"});
}
export async function thumbnail(file, width, height) {
    const bitmap = await createImageBitmap(file);
    try {
        const canvas = document.createElement("canvas");
        canvas.width = width; canvas.height = height;
        const ctx = canvas.getContext("2d", {willReadFrequently: true});
        const scale = Math.min(width / bitmap.width, height / bitmap.height, 1);
        const w = Math.max(1, Math.round(bitmap.width * scale));
        const h = Math.max(1, Math.round(bitmap.height * scale));
        ctx.drawImage(bitmap, Math.floor((width-w)/2), Math.floor((height-h)/2), w, h);
        return new Uint8Array(ctx.getImageData(0,0,width,height).data.buffer);
    } finally { bitmap.close(); }
}
async function mediaBytes(worker, path, encrypted, limit) {
    const channel = new MessageChannel();
    return await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {channel.port1.close(); reject(new Error("Download timed out"));},120000);
        channel.port1.onmessage = ({data}) => {
            clearTimeout(timer); channel.port1.close();
            if (data instanceof Uint8Array && (!limit || data.length <= limit)) resolve(data); else reject(new Error("Download failed"));
        };
        worker.postMessage({command:"untethered-download",path,encrypted,limit},[channel.port2]);
    });
}
export async function inline_blob(worker,path,encrypted,limit) {
    return new Blob([await mediaBytes(worker,path,encrypted,limit)]);
}
export async function save_media(worker, path, encrypted, name) {
    const bytes=await mediaBytes(worker,path,encrypted,0);
    const url = URL.createObjectURL(new Blob([bytes],{type:"application/octet-stream"}));
    const link=document.createElement("a");link.href=url;link.download=name || "attachment";
    document.body.append(link);link.click();link.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);
}
