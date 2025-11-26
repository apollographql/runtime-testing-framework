# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_rtf_global_optspecs
	string join \n var= vars= v/verbose h/help
end

function __fish_rtf_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_rtf_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_rtf_using_subcommand
	set -l cmd (__fish_rtf_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c rtf -n "__fish_rtf_needs_command" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_needs_command" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_needs_command" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_needs_command" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_needs_command" -f -a "run" -d 'Check and run a test plan'
complete -c rtf -n "__fish_rtf_needs_command" -f -a "expand-matrix" -d 'Expand a test plan matrix into JSON'
complete -c rtf -n "__fish_rtf_needs_command" -f -a "template" -d 'Template a test plan using provided variables, outputting the resulting config to stdout'
complete -c rtf -n "__fish_rtf_needs_command" -f -a "custom-provider" -d 'Work directly with custom file provider definitions'
complete -c rtf -n "__fish_rtf_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c rtf -n "__fish_rtf_using_subcommand run" -l github -d 'Execute a test plan file in GitHub instead of from a local path' -r
complete -c rtf -n "__fish_rtf_using_subcommand run" -l ref -d 'Optional git ref to pull files from when using --github' -r
complete -c rtf -n "__fish_rtf_using_subcommand run" -l outdir -d 'Output directory for providers when they run' -r
complete -c rtf -n "__fish_rtf_using_subcommand run" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand run" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand run" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand run" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand expand-matrix" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand expand-matrix" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand expand-matrix" -s c -l compact -d 'Return the expanded matrix JSON in compact form'
complete -c rtf -n "__fish_rtf_using_subcommand expand-matrix" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand expand-matrix" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand template" -l github -d 'Template a test plan file in GitHub instead of from a local path' -r
complete -c rtf -n "__fish_rtf_using_subcommand template" -l ref -d 'Optional git ref to pull files from when using --github' -r
complete -c rtf -n "__fish_rtf_using_subcommand template" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand template" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand template" -l check -d 'Run a static check of the resulting test plan after templating'
complete -c rtf -n "__fish_rtf_using_subcommand template" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand template" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -f -a "template" -d 'Template a custom provider definition, outputting the resulting config to stdout'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -f -a "run" -d 'Execute a custom provider definition'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and not __fish_seen_subcommand_from template run help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from template" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from template" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from template" -l check -d 'Run a static check of the resulting test plan after templating'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from template" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from template" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from run" -l outdir -d 'Output directory for provider execution' -r
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from run" -l var -d 'A single additional templating variable in the form "key=value"' -r
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from run" -l vars -d 'Path to a JSON file containing additional template variables' -r -F
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from run" -s v -l verbose -d 'Flag to control logging verbosity. Default level is `warn`. `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from run" -s h -l help -d 'Print help'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from help" -f -a "template" -d 'Template a custom provider definition, outputting the resulting config to stdout'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from help" -f -a "run" -d 'Execute a custom provider definition'
complete -c rtf -n "__fish_rtf_using_subcommand custom-provider; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c rtf -n "__fish_rtf_using_subcommand help; and not __fish_seen_subcommand_from run expand-matrix template custom-provider help" -f -a "run" -d 'Check and run a test plan'
complete -c rtf -n "__fish_rtf_using_subcommand help; and not __fish_seen_subcommand_from run expand-matrix template custom-provider help" -f -a "expand-matrix" -d 'Expand a test plan matrix into JSON'
complete -c rtf -n "__fish_rtf_using_subcommand help; and not __fish_seen_subcommand_from run expand-matrix template custom-provider help" -f -a "template" -d 'Template a test plan using provided variables, outputting the resulting config to stdout'
complete -c rtf -n "__fish_rtf_using_subcommand help; and not __fish_seen_subcommand_from run expand-matrix template custom-provider help" -f -a "custom-provider" -d 'Work directly with custom file provider definitions'
complete -c rtf -n "__fish_rtf_using_subcommand help; and not __fish_seen_subcommand_from run expand-matrix template custom-provider help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c rtf -n "__fish_rtf_using_subcommand help; and __fish_seen_subcommand_from custom-provider" -f -a "template" -d 'Template a custom provider definition, outputting the resulting config to stdout'
complete -c rtf -n "__fish_rtf_using_subcommand help; and __fish_seen_subcommand_from custom-provider" -f -a "run" -d 'Execute a custom provider definition'
