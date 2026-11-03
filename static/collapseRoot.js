// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// "[collapse root]" links on nested comments: collapse the thread's root
// comment and scroll to it.

// Unhides the [collapse root] links rendered by comment.html.
document.body.classList.add("js-enabled");

// Delegated so links in comments fetched by loadMoreComments.js keep working.
document.addEventListener("click", (event) => {
    const link = event.target.closest("a.collapse_root");
    if (!link) return;
    event.preventDefault();
    event.stopPropagation();

    // The top-level ancestor comment: its .comment ancestor chain ends inside
    // the .thread wrapper.
    let root = link.closest(".comment");
    while (root.parentElement?.closest(".comment")) {
        root = root.parentElement.closest(".comment");
    }
    // Only the direct-child details; nested replies keep their own state.
    root.querySelector(":scope > .comment_right")?.removeAttribute("open");
    root.scrollIntoView({ behavior: "smooth", block: "start" });
});
// @license-end
