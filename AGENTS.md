# Working on Untethered as an agent

Several agents (Claude Code, opencode, others) often work on this repository at the same
time, each in its own git worktree and branch, on this machine or others. They coordinate
through the **work board**: a shared list of who is doing what, on which branch, touching
which files. It lives on its own branch, `work-board`, which nobody checks out: every
worktree sees a change the moment it's made, and the tool syncs it with GitHub on every
read and change. `scripts/work.py` is the only way to change it; anyone can read it on
GitHub as [`BOARD.md` on the `work-board` branch](https://github.com/koalexbear/untethered/blob/work-board/BOARD.md).

The project itself: [README.md](README.md) for what exists and how to run it,
[ROADMAP.md](ROADMAP.md) for what's next (its step IDs, such as `M2`, are task IDs here),
[rust-client/docs/](rust-client/docs/) for design notes.

## The rules

1. **Look before you start.** Run `scripts/work.py` (Claude sessions see it at start).
   Don't pick up a task that's on the board, and don't change files another claim lists
   unless you've coordinated (rule 4).
2. **Claim before you write code.** One claim per task, from your own worktree:

   ```sh
   scripts/work.py claim M2 --title "Composer autocomplete" \
       --paths rust-client/src/view.rs rust-client/src/view/composer.rs
   ```

   Use the ROADMAP step ID, or a short slug for anything else (`fix-scrollbar-drag`).
   List the files or directories you expect to change; globs work. The claim records
   your branch; name yourself with `--by NAME` or `WORK_AGENT=NAME` if the branch name
   isn't clear (for example your session's name). If the claim is refused, someone has
   it: pick something else or ask the user.
3. **Keep it current.** `scripts/work.py touch ID` when you commit or every few hours (a
   claim untouched for 24 hours shows as STALE). Add files you turn out to need with
   `touch ID --paths …`; change status with `--status blocked|review|paused`.
4. **Overlap means talking, not taking.** If your paths overlap someone's claim (big
   shared files like `view.rs` and `main.rs` often do), `claim` stops and tells you who.
   Leave them a note saying what you'll touch, then claim with
   `--share "what you'll change there and why"`; they get that note too. Keep shared
   edits small and in clearly separate places.
5. **Talk through notes.** `scripts/work.py note ID "…"` leaves a message on any claim;
   `scripts/work.py show ID` reads them all. Check notes on your own claim when you touch
   it and before you finish.
6. **Check before you commit or hand over.** `scripts/work.py check` lists files your
   branch changed that another claim covers (exit status 1 if any) and files outside
   your own claim. Resolve those first.
7. **Finish cleanly.** When your branch is merged: `scripts/work.py done ID --result
   "merged as <sha>"`. If you stop without finishing: `scripts/work.py release ID
   --result "why, and what's left"`.
8. **Never take someone else's work.** Don't edit, commit to, rebase, push or delete
   another agent's branch or worktree. Don't take over a claim, even a stale one, unless
   the user says so (`claim ID --take-over`). Don't use `--force` on someone else's
   claim without the user's say-so.

## Branches and landing

- Work on your own branch in your own worktree; commit there. Don't commit to or push
  `main` unless the user asks you to land your work.
- Before landing, rebase or merge the latest `main`, run the checks below, and run
  `scripts/work.py check`.
- The user (or an agent they ask) merges to `main` and pushes. Then mark the claim done.

## Checks before handing work back

From `rust-client/`:

```sh
cargo fmt --check
cargo clippy --all-targets          # only the existing rooms.rs warning is expected
cargo test
cargo check --target wasm32-unknown-unknown
cargo check --target x86_64-pc-windows-gnu
```

For UI changes, also look at the running client; the QA fixtures under `scripts/` drive it
(see the README's testing notes).

## Working from other machines

Every clone shares the board through `origin` with no setup: each read fetches it, and
each change is pushed. The push decides a race, so two machines can't both win a claim.
Offline, changes stay local and are pushed (merged if needed) on the next read.

- **Push access is needed to claim.** Without it (working from a fork, say) the board
  still reads fine, but your claims stay local and the tool says so. Ask the owner for
  write access, or ask someone who has it to claim for you.
- Credentials: the tool uses your normal git credentials, and falls back to the GitHub
  CLI (`gh auth login`) once if those don't get through. It never waits on a prompt.
- `git config workboard.remote NAME` uses another remote; `none` keeps the board local.
