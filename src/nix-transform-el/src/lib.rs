use std::fmt::Display;

use emacs_module::{
    internal::{emacs_env, emacs_value},
    EmacsEnv,
};
use nix_transform::{update_fetcher, UpdateFetcherError};

#[no_mangle]
static plugin_is_GPL_compatible: u32 = 0;

#[derive(Debug)]
#[allow(dead_code)]
enum Error {
    BufferNotUtf8,
    UpdateFetcherError(UpdateFetcherError),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BufferNotUtf8 => write!(f, "Buffer is not valid UTF8 sequence"),
            Error::UpdateFetcherError(update_fetcher_error) => update_fetcher_error.fmt(f),
        }
    }
}

fn update_fetcher_impl(env: EmacsEnv) -> Result<(), Error> {
    let buffer_str = env.fun_call(
        env.intern(c"buffer-substring-no-properties"),
        &[
            env.fun_call(env.intern(c"point-min"), &[]),
            env.fun_call(env.intern(c"point-max"), &[]),
        ],
    );
    let buffer_str = env
        .copy_string_to_string(buffer_str)
        .map_err(|_| Error::BufferNotUtf8)?;

    let curr_point = env.fun_call(env.intern(c"point"), &[]);
    let point_offset = env.fun_call(env.intern(c"position-bytes"), &[curr_point]);
    let point_offset = env.extract_integer(point_offset) as usize - 1;

    let update = update_fetcher(&buffer_str, point_offset).map_err(Error::UpdateFetcherError)?;

    env.fun_call(
        env.intern(c"kill-region"),
        &[
            env.make_integer(update.modification.prefix_offset as i64 + 2), // point starts at 1 and skip "
            env.make_integer(update.modification.suffix_offset as i64),
        ],
    );
    env.fun_call(
        env.intern(c"goto-char"),
        &[env.make_integer(update.modification.prefix_offset as i64 + 2)], // point starts at 1 and skip "
    );
    env.fun_call(
        env.intern(c"insert"),
        &[env.make_string(update.modification.to_insert.as_bytes())],
    );
    env.fun_call(env.intern(c"goto-char"), &[curr_point]);

    Ok(())
}

extern "C" fn update_fetcher_el(
    env: *mut emacs_env,
    _n_args: isize,
    _args: *mut emacs_value,
    _data: *mut core::ffi::c_void,
) -> emacs_value {
    let env = EmacsEnv::from_env(env);
    match update_fetcher_impl(env) {
        Ok(()) => env.intern(c"t"),
        Err(err) => {
            let error = env.intern(c"user-error");
            let msg = env.make_string(err.to_string().as_bytes());
            env.fun_call(error, &[msg])
        }
    }
}

#[no_mangle]
extern "C" fn emacs_module_init(runtime: *mut emacs_module::internal::emacs_runtime) -> u32 {
    let env = EmacsEnv::from_runtime(runtime);

    let f = env.create_function(
        c"nix-transform-update-fetcher",
        0,
        0,
        update_fetcher_el,
        cr#"Update fetcher at point

(fn)"#,
    );
    env.make_interactive(f);
    env.provide(c"nix-transform");

    return 0;
}
