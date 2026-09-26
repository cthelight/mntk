
move_to_here_in_order() {
	local dest="$PWD"
	local dir
	local dircnt=0
	for dir in "$@"
	do
		((dircnt++))
		pushd "$dir"
		for file in *
		do
			mv "$file" "$dest/${dircnt}_${file}"
		done
		popd
	done
}

rename_seq_ep() {
	local season="$1"
	local epnum=0
	local file
	for file in *
	do
		((epnum++))
		mv "$file" "$(printf "Episode S%02dE%02d.mkv" "$season" "$epnum")"
	done
}
