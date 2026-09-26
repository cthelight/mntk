#! /usr/bin/env bash

###############################
# OMDB_ID _MUST_ be defined!!!
###############################


INVALID_CHARS=':/*"<>|?'
SUB_CHAR='_'

omdb_query() {
    local args_to_curl="$1"
    args_to_curl="$(tr ' ' '+' <<< "$args_to_curl")"
    curl -s "http://www.omdbapi.com/?apikey=$OMDB_ID&$args_to_curl"
}


rename_movie_imdb(){
    local mkv_file="$1"
    local imdb_id="$2"

    local movie_data="$(omdb_query "i=$imdb_id")"
    local name
    echo "$movie_data"
    if [ "$(jq '.Response' <<< "$movie_data")" != "True" ]; then
            name="$(\
                    jq '.Title, .Year' <<< "$movie_data" \
                    | xargs printf "%s (%s)\n" \
                    | tr "$INVALID_CHARS" "$SUB_CHAR"\
                    )"
    else
            1>&2 echo "Movie not found!"
    fi
    mkdir -p "$name"
    mv -n "$mkv_file" "$name/$name.mkv"
}


omdb_movie_search() {
    local results="$(omdb_query "s=$1")"
    local choice
    choice="$(\
            jq '.Search[] | .imdbID, .Title, .Year' <<< "$results"\
            | xargs -n 3 printf '"%s" "%s (%s)"\n'\
            | xargs dialog --stdout --keep-tite --menu "test" 22 100 100\
            )"
    if [ -z "$choice" ]; then
            return 1
    fi
    echo "$choice"
}

movie_search_and_rename_single() {
    local movie_path="$1"
    local movie_path_full
    local search_param
    local imdb_id
    if ! [ -f "$movie_path" ]; then
            1>&2 echo "Provided path not a file. Exiting"
            return 1
    fi
    movie_path_full="$(realpath "$movie_path")"

    search_param="$(dialog --stdout --keep-tite --inputbox "Title search for: $movie_path" 22 100)"
    if imdb_id="$(omdb_movie_search $search_param)"; then
            rename_movie_imdb "$movie_path_full" "$imdb_id"
            echo "Moved $movie_path!"
    else
            1>&2 echo Search failed. Aborting!
            return 1
    fi
}


