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
export function pick_avatar(request) {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/jpeg,image/gif,image/webp";
    input.onchange = input.oncancel = () => stage(request, input.files);
    input.click();
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
export function save_bytes(bytes, name) {
    const url = URL.createObjectURL(new Blob([bytes],{type:"application/octet-stream"}));
    const link=document.createElement("a");link.href=url;link.download=name || "attachment";
    document.body.append(link);link.click();link.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);
}
// The media viewer's video in the browser (ROADMAP P4): a native player over
// the viewer's stage, between its header and its strip and caption.
let video, videoUrl;
export function show_video(bytes, name) {
    hide_video();
    const type = /\.webm$/i.test(name) ? "video/webm" : /\.ogv$/i.test(name) ? "video/ogg" : "video/mp4";
    videoUrl = URL.createObjectURL(new Blob([bytes], {type}));
    video = document.createElement("video");
    video.id = "untethered-video";
    video.controls = true; video.autoplay = true; video.playsInline = true;
    Object.assign(video.style, {position: "fixed", left: "72px", right: "72px", top: "64px",
        bottom: (video_bottom || 40) + "px", width: "calc(100% - 144px)", objectFit: "contain",
        background: "transparent", zIndex: 10});
    video.style.height = `calc(100% - ${64 + (video_bottom || 40)}px)`;
    video.src = videoUrl;
    document.body.append(video);
}
let video_bottom = 40;
export function place_video(bottom) {
    video_bottom = bottom;
    if (video) {
        video.style.bottom = bottom + "px";
        video.style.height = `calc(100% - ${64 + bottom}px)`;
    }
}
export function hide_video() {
    if (video) { video.pause(); video.remove(); video = null; }
    if (videoUrl) { URL.revokeObjectURL(videoUrl); videoUrl = null; }
}
