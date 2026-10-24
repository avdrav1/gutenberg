#!/bin/sh
# Plays the recorded Hugging Face MCP server over stdio: each request gets
# the recorded reply for its method, carrying the request's id.
# Notifications get nothing. Exits when stdin closes.
dir=$(dirname "$0")
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) reply=huggingface-initialize.json ;;
    *'"method":"tools/list"'*) reply=huggingface-tools-list.json ;;
    *) continue ;;
  esac
  # The JSON escapes its newlines, so joining the lines keeps it valid.
  tr -d '\n' < "$dir/$reply" | sed "s/\"id\": [0-9]*}\$/\"id\": $id}/"
  echo
done
