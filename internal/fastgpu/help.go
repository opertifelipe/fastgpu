package fastgpu

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"
)

func configureHelp(root *cobra.Command) {
	root.SetHelpCommand(&cobra.Command{Use: "help [command]", Short: "Show help for a command", RunE: func(_ *cobra.Command, args []string) error {
		cmd, rest, err := root.Find(args)
		if err != nil {
			return err
		}
		if len(rest) > 0 {
			return fmt.Errorf("unknown command: %s", strings.Join(args, " "))
		}
		return cmd.Help()
	}})
	root.SetUsageTemplate(`Usage:
  {{.UseLine}}{{if .HasAvailableSubCommands}}
  {{.CommandPath}} [command]{{end}}{{if .HasAvailableSubCommands}}

Available Commands:{{range .Commands}}{{if (or .IsAvailableCommand (eq .Name "help"))}}
  {{rpad .Name .NamePadding }} {{.Short}}{{end}}{{end}}{{end}}{{if .HasAvailableLocalFlags}}

Flags:
{{.LocalFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableInheritedFlags}}

Global Flags:
{{.InheritedFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableSubCommands}}

Use "{{.CommandPath}} [command] --help" for more information about a command.
{{end}}
`)
	root.PersistentFlags().BoolP("help", "h", false, "Show help")
	root.InitDefaultVersionFlag()
	root.Flags().Lookup("version").Usage = "Show version"
	root.CompletionOptions.DisableDefaultCmd = true
}
