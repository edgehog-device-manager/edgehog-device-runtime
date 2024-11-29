# edgehog-device-runtime-containers-store

Separate crate with the persistent state for the container service.

This is in its own separate crate to reduce the compile times for the main container service.

## Tools

It uses the [diesel](https://docs.rs/diesel/latest/diesel/) ORM to generate safe SQL queries.

To generate new migration you'll need the `diesel-cli`

```sh
export DATABASE_URL=sqlite:///tmp/edgehog-containers/state.db
diesel migration generate create_some_migration
```

There are also script to run in the `scripts` directory to generate the SQLite file or run all the
migrations.
