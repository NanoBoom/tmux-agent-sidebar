---
title: Keybindings
description: Every shortcut in the sidebar, the worktree spawn and open modals, and the close-pane modal.
---

## Sidebar

| Key            | Action                                                        |
| -------------- | ------------------------------------------------------------- |
| `prefix + e`   | Toggle sidebar                                                |
| `prefix + E`   | Toggle sidebar in all windows                                 |
| `j` / `Down`   | Move selection down                                           |
| `k` / `Up`     | Move selection up                                             |
| `h` / `Left`   | Previous status filter                                        |
| `l` / `Right`  | Next status filter                                            |
| `r`            | Open repo filter popup                                        |
| `Enter`        | Jump to the selected pane                                     |
| `Tab`          | Cycle status filter                                           |
| `Shift+Tab`    | Switch bottom panel tab (Activity ⇄ Git)                      |
| `Esc`          | Return focus or close the popup                               |

## Repo filter popup

Opened with `r` or by clicking the repo filter button in the sidebar header.

| Key           | Action                                 |
| ------------- | -------------------------------------- |
| `j` / `Down`  | Move selection down                    |
| `k` / `Up`    | Move selection up                      |
| `Enter`       | Confirm — filter the list to that repo |
| `Esc`         | Cancel                                 |

## Notices popup

Opened by clicking the `ⓘ` badge shown when hooks or plugin setup are missing.

| Key   | Action          |
| ----- | --------------- |
| `Esc` | Close the popup |

## Worktree

| Key | Action                                       |
| --- | -------------------------------------------- |
| `n` | Spawn a new worktree + agent                 |
| `o` | Open an existing worktree with an agent      |
| `x` | Close the selected sidebar-created pane      |

## Spawn worktree modal

Opened with `n` on a repo.

| Key                                | Action                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------------------ |
| Text keys                          | Type the name (used as the branch slug and tmux window name)                                     |
| `↑` / `↓` / `Tab` / `Shift+Tab`    | Move focus between `NAME` / `AGENT` / `MODE` fields                                              |
| `←` / `→`                          | Cycle the value when the agent or mode field has focus                                           |
| `Enter`                            | Create the worktree + window and launch the agent                                                |
| `Esc`                              | Cancel                                                                                           |

## Open worktree modal

Opened with `o` on a pane. Lists every existing worktree of that pane's
repository — the main checkout included — and launches an agent in the
one you pick. Unlike `n` this creates no git state, but the resulting
window is closed with `x` just like a spawned one — including the `[y]`
option, which will delete a worktree and branch you created yourself
(refused while that worktree has uncommitted changes).

### Step 1 — pick the worktree

A `●` in front of a row means a pane is already running in that
worktree. It is informational: `Enter` still opens a new window.

| Key                                 | Action                          |
| ----------------------------------- | ------------------------------- |
| `j` / `k` / `↑` / `↓` / `Ctrl+n` / `Ctrl+p` | Move selection          |
| `Enter`                             | Confirm and go to step 2        |
| `Esc`                               | Close the modal                 |

### Step 2 — pick the agent and mode

`BRANCH` is read-only — it shows what you picked in step 1.

| Key                              | Action                                        |
| -------------------------------- | --------------------------------------------- |
| `↑` / `↓` / `Tab` / `Shift+Tab`  | Move focus between `AGENT` / `MODE`           |
| `←` / `→`                        | Cycle the focused value                       |
| `Enter`                          | Open a window in the worktree and launch      |
| `Esc`                            | Back to step 1                                |

## Close pane modal

Opened with `x` on a sidebar-created pane (`n` or `o`).

| Key             | Action                                                                                                    |
| --------------- | --------------------------------------------------------------------------------------------------------- |
| `y` / `Enter`   | Close the tmux window, remove the git worktree (`--force`), and delete the branch (`git branch -D`)       |
| `c`             | Close the tmux window only, keep the worktree and branch on disk                                          |
| `n` / `Esc`     | Cancel                                                                                                    |

`y` / `Enter` are **refused while the worktree has uncommitted changes**
— the modal shows `! uncommitted changes`, renders the option as
`[y] remove — blocked`, and answers `commit or stash first` if you press
it anyway. `c` keeps working. See
[Uncommitted changes block `[y]`](/tmux-agent-sidebar/features/worktree/#uncommitted-changes-block-y).
