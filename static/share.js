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
    try {
        field.select();
        return document.execCommand("copy") ? Promise.resolve() : Promise.reject();
    } catch (error) {
        return Promise.reject(error);
    } finally {
        field.remove();
    }
}

// When copying fails too, show the address to copy by hand instead of leaving the page.
function copyLink(link) {
    copy(link.href).then(
        () => {
            link.classList.add("copied");
            clearTimeout(link.copiedTimer);
            link.copiedTimer = setTimeout(() => link.classList.remove("copied"), 2000);
        },
        () => prompt("Copy this link:", link.href),
    );
}

document.addEventListener("click", (event) => {
    const link = event.target.closest("a.share");
    if (!link) return;
    event.preventDefault();
    if (navigator.share) {
        navigator.share({ url: link.href, title: document.title }).catch((error) => {
            if (error.name !== "AbortError") copyLink(link);
        });
        return;
    }
    copyLink(link);
});
// @license-end
