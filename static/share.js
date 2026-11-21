// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
document.addEventListener("click", (event) => {
    const link = event.target.closest("a.share");
    if (!link || !(navigator.share || navigator.clipboard)) return;
    event.preventDefault();
    if (navigator.share) {
        navigator.share({ url: link.href, title: document.title }).catch(() => {});
        return;
    }
    navigator.clipboard.writeText(link.href).then(() => {
        link.classList.add("copied");
        clearTimeout(link.copiedTimer);
        link.copiedTimer = setTimeout(() => link.classList.remove("copied"), 2000);
    });
});
// @license-end
