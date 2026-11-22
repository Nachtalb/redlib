// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Browsers offer the share sheet and the clipboard on https and localhost only: elsewhere the
// address is copied the old way, from a selected field.
function copy(text) {
    if (navigator.clipboard) return navigator.clipboard.writeText(text);
    const field = document.createElement("textarea");
    field.value = text;
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.append(field);
    field.select();
    const copied = document.execCommand("copy");
    field.remove();
    return copied ? Promise.resolve() : Promise.reject();
}

document.addEventListener("click", (event) => {
    const link = event.target.closest("a.share");
    if (!link) return;
    event.preventDefault();
    if (navigator.share) {
        navigator.share({ url: link.href, title: document.title }).catch(() => {});
        return;
    }
    copy(link.href).then(
        () => {
            link.classList.add("copied");
            clearTimeout(link.copiedTimer);
            link.copiedTimer = setTimeout(() => link.classList.remove("copied"), 2000);
        },
        () => location.assign(link.href),
    );
});
// @license-end
