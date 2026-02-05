<!-- diataxis-type: howto -->

# Generating CLI shell completions

The `rtf completion -s <shell>` command can be used to generate shell completion scripts for your
shell of choice.

To set up completions manually, follow the instructions below. The exact config file locations might
vary based on your system. Make sure to restart your shell before testing whether completions are
working.

## bash

First, ensure that you install `bash-completion` using your package manager.

After, add this to your ~/.bash_profile:

```sh
eval "$(rtf completion -s bash)"
```

## zsh

Generate an `_rtf` completion script and put it somewhere in your `$fpath`:

```sh
rtf completion -s zsh > /usr/local/share/zsh/site-functions/_rtf
```

Ensure that the following is present in your `~/.zshrc`:

```sh
autoload -U compinit
compinit -i
```

## fish

Generate an `rtf.fish` completion script:

```sh
rtf completion -s fish > ~/.config/fish/completions/rtf.fish
```
