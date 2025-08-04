when --override-identical is used, it makes no sense to have this:

↻ override identical: ~/.vimrc <- ~/Developer/dotfiles/dotty/.vimrc
✔ Linked ~/.vimrc -> ~/Developer/dotfiles/dotty/.vimrc

because the file content is of course the same
so check if the symlink is already same

and yeah, lets add --verbose!
On non verbose:
- print conflicts and skipped by lua
- print override identical
