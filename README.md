# CBTR

Check Build Test Run

## Motivation

As a developer, I run commands like `cargo build`, `docker build`, `poetry run`, etc all the time. Basically, it's all variations of `<tool> format|check|build|test|run`, maybe with an extra flag here and there.
I noticed this pattern and started to write makefiles with `.phony` targets and then my workflow became
`make build; make test; make run`. With some of my projects using [justfiles](TODO) instead of make files, I now had two new tools repeating the same pattern of `<tool> format|check|build|test|run`.
That's a lot of keystrokes over time.

I started to use some shell aliases `alias b='just build'; alias t='just test'; alias r='just run'`, which worked great until I had to work with a project without a justfile.
Muscle memory can be a double edged sword.

```console
$ b
error: No justfile found

$ fsck
^C

$ cargo build

```

I realized I wanted a smarter alias that would enact the appropriate tool depending on the context.

- Justfile found? Run `just build`
- Cargo.toml? Run `cargo build`
- pyproject.toml? `poetry build`

At some point along the way I added `c` because I run `cargo check` a fair amount.
This became `cbtr` for Check, Build, Test Run.
cbtr also supports `f` for commands like `cargo fmt`, but I had already settled on the name 'cbtr' when I added 'f' ¯\\_(ツ)_/¯.

## Usage

This crate provides binary called `cbtr`. Running `cbtr <command>`, as you might have noticed, is yet another `<tool> <command>` for our toolbox. But, `cbtr` is a
a [multicall binary](https://www.busybox.net/BusyBox.html).

Instead of running `cbtr` directly, hard/symlink the binary to 'f', 'c', 'b', 't', and 'r' and run those instead.

```console
$ ln -s ~/.local/bin/f ./cbtr
$ ln -s ~/.local/bin/c ./cbtr
$ ln -s ~/.local/bin/b ./cbtr
$ ln -s ~/.local/bin/t ./cbtr
$ ln -s ~/.local/bin/r ./cbtr
```

These links are called "applets", a term from [busybox](https://www.busybox.net/BusyBox.html#usage), the most widely known multicall program.

Now, running `b` invokes `cbtr b`, saving us the painstaking process of typing out 'cbtr ' each time. Think of the keystroke savings!

(Fyi: you can also make aliases (e.g `alias b='cbtr b'`) if you prefer. I like the multicall applets because it's shell agnostic and I tend to switch between zsh and nushell).

All that is left now is to create a config file.

## Configuring

cbtr rules are defined in a toml file at `${XDG_CONFIG_HOME}/cbtr/config.toml`.
The config file is an array of "entries", where each entry defines a set of tools to be ran based on some conditions.

A repo may also contain a config file in the repo root (directory containing the .git dir). The repo-level configuration will be checked first and then the user-level config will be fallen back on.

Here is an example entry with the general concepts outlined. More in-dept explnation will follow.

```toml
[[entry]]
name = "rust"
bin = "cargo" # These rules only apply if 'cargo' is found on the $PATH
file.name = "Cargo.toml" # These rules only apply if a 'Cargo.toml' exists in the repo
file.search-direction = "backwards" # Search from CWD backwards to the repo root
tools.format = "cargo fmt" # 'f' will become a shortcut for 'cargo fmt' 
tools.build = "cargo build" # b -> cargo build
tools.test = "cargo test" # t -> cargo test
tools.run = "cargo run" # r -> cargo run
tools.check = ["cargo check", "cargo clippy"] # Multiple tools may be ran, in the specified order. If any fail, the rest will not be ran
```

There are two types of conditions

- file: One or more files must exist in the repo for the entry to be selected. "Repo" refrs to a git repository.
  The root of the repo is the directory containing the '.git' dir. If the current directory is part of in a git repo, then files are only searched for in CWD.
  - file.name: The name of the file to search the repo for
  - file.search-direction: The direction to search the repo for the named file. Options are "forwards" or "backwards". Defaults to "backwards"
    - forwards: Search starting with the repo's root dir, forwards to the CWD
    - backwards: Search starting with the CWD, backwards to the repo root
- bin: One or more commands must exist in the $PATH.

The order of the entries in the config file matters. The conditions are checked from top to bottom and the first matching entry's tool is used.
If the conditions for a particular entry are met, but that entry doesn't define a tool for the invoked applet, the next matching entry (with the appropriate tool defined) will be used.

The config I use daily is included in the repo as an example: [config.toml](./example-config.toml).

## Future

None of this is guarentted, but I have a few thoughts on possible directions.

- Why be limited by f,c,b,t,r? What if this program could be invoked as an applet of any letter? It wouldn't be hard to change the config options from `tool.check` to `tool.c` and allow for `tool.[a-z]`.
- Searching 'forwards' for a file in a repo for a command such as 'just' doesn't really mean anything right now. 'just' itself searches *backwards* for a justfile. I.e. The first justfile found by 'cbtr' may not be the one used by 'just'.
  This doesn't meaningfully affect the program's behavior (cbt doesn't read any justfile), but this is something to consider. In the future, I'd like cbtr to pass a flag to just, teling just which justfile to use, thus forwards/backwards would matter.
- Have an option in the config to set env vars.
- Global config options
- Ability to pass flags to the applets when then get forwarded on to the underlying tool
  example: `b --all` -> `cargo build --all`

Please open an issue if any of these (or something else??) appeals to you and I'll try to prioritize it!

## Prior Art

`cbtr` was originally inspired by a project called 't-for-test'. As the name implies, t-for-test provided a binary called `t` that acted as a shortcut for running commands like `cargo test`, `pytest`, etc.
Later t-for-test was renamed to 'project-do' and expanded to support for `b` and `r` shortcuts. My issue with project-do is that all the commands wrapped by project-do were all hard-coded.
I wanted more flexibility with a configuration scheme which lead me to develop cbtr.

Unfortunately, I can't find a link to project-do. If someone knows where to find it, please let me know! It's been a few years now, so maybe it's more flexible.
