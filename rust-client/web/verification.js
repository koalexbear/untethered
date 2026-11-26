// Keep the editable value in a real, masked HTML input. A normal user paste
// then works without asynchronously reading the system clipboard.
// Looked up on first use: the matrix-sdk engine's worker imports this module
// too, and a worker has no document.
let dialog, form, input, button, feedback;
function elements() {
    if (dialog) return;
    dialog = document.getElementById("verify-dialog");
    form = document.getElementById("verify-form");
    input = document.getElementById("verify-secret");
    button = form.querySelector("button");
    feedback = document.getElementById("verify-feedback");
}
let available = false;
let submitting = false;

function updateButton() {
    button.disabled = !available || submitting || !input.value.trim();
}
export function install(submit) {
    elements();
    input.addEventListener("input", updateButton);
    dialog.addEventListener("cancel", event => event.preventDefault());
    form.addEventListener("submit", event => {
        event.preventDefault();
        if (button.disabled) return;
        submitting = true;
        updateButton();
        submit(input.value);
    });
}
export function render(show, user, enabled, busy, status) {
    elements();
    if (!show) {
        if (dialog.open) dialog.close();
        input.value = "";
        return;
    }
    if (!dialog.open) {
        dialog.showModal();
        input.focus();
    }
    document.getElementById("verify-user").textContent = user;
    available = enabled;
    submitting = busy;
    input.readOnly = busy;
    updateButton();
    button.textContent = busy ? "Verifying…" : "Verify device";
    if (feedback.textContent !== status) feedback.textContent = status;
}
