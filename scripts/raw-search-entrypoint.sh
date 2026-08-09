#!/bin/sh
set -eu

chown rawsearch:rawsearch /data/mutable
exec gosu rawsearch youtube-raw-search serve-mutable
