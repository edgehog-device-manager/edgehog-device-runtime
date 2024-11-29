#!/usr/bin/env bash

set -exEuo pipefail

dbfile=$(echo "$DATABASE_URL" | sed -e's|^sqlite://||')

sqlite3 "$dbfile" <assets/init.sql
