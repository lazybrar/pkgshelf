set -l cmds add rm list check diff update hold unhold rollback doctor migrate setup help
complete -c pkgshelf -f
complete -c pkgshelf -n "not __fish_seen_subcommand_from $cmds" -a "$cmds"
complete -c pkgshelf -n "__fish_seen_subcommand_from add" -a "aur local"
complete -c pkgshelf -n "__fish_seen_subcommand_from rm diff update hold unhold rollback" -a "(pkgshelf list 2>/dev/null | awk '{n=\$2; sub(\".*/\",\"\",n); print n}')"
complete -c pkgshelf -n "__fish_seen_subcommand_from update" -l yes -l force -l devel
complete -c pkgshelf -n "__fish_seen_subcommand_from rm" -l purge
complete -c pkgshelf -n "__fish_seen_subcommand_from check" -l quiet
