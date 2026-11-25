// Keep the editable value in a real, masked HTML input. A normal user paste
// then works without asynchronously reading the system clipboard.
const dialog = document.getElementById("verify-dialog");
const form = document.getElementById("verify-form");
const input = document.getElementById("verify-secret");
const button = form.querySelector("button");
const feedback = document.getElementById("verify-feedback");
let available = false;
let submitting = false;

function updateButton() {
    button.disabled = !available || submitting || !input.value.trim();
}
export function install(submit) {
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
export function render(show, user, enabled, busy, status, gomuks) {
    // The note is about gomuks's verification; the matrix-sdk engine has none.
    document.getElementById("verify-gomuks-note").hidden = !gomuks;
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
