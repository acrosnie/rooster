// Copyright 2014-2017 The Rooster Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// #![allow(useless_format, too_many_arguments)]

extern crate libc;
extern crate getopts;
extern crate rustc_serialize;
extern crate crypto;
extern crate rpassword;
extern crate rand;
extern crate byteorder;

use std::fs::File;
use std::env;
use std::env::VarError;
use std::path::MAIN_SEPARATOR as PATH_SEP;
use std::io::Result as IoResult;
use std::io::Error as IoError;
use std::io::ErrorKind as IoErrorKind;
use std::io::Write;
use std::io::Read;
use std::path::{Path, PathBuf};
use getopts::Options;
use rpassword::prompt_password_stderr;
use safe_string::SafeString;
use safe_vec::SafeVec;
use std::ops::Deref;

mod macros;
mod aes;
mod commands;
mod ffi;
mod password;
mod color;
mod safe_string;
mod safe_vec;
mod generate;
mod clipboard;

const ROOSTER_FILE_ENV_VAR: &'static str = "ROOSTER_FILE";
const ROOSTER_FILE_DEFAULT: &'static str = ".passwords.rooster";
const DONT_CREATE_PASSWORD_FILE: &'static str = "DONT_CREATE_PASSWORD_FILE";
const FAIL_READING_NEW_PASSWORD: &'static str = "FAIL_READING_NEW_PASSWORD";

struct Command {
    name: &'static str,
    callback_exec: fn(&getopts::Matches, &mut password::v2::PasswordStore) -> Result<(), i32>,
    callback_help: fn(),
}

static COMMANDS: &'static [Command] =
    &[Command {
          name: "get",
          callback_exec: commands::get::callback_exec,
          callback_help: commands::get::callback_help,
      },
      Command {
          name: "add",
          callback_exec: commands::add::callback_exec,
          callback_help: commands::add::callback_help,
      },
      Command {
          name: "delete",
          callback_exec: commands::delete::callback_exec,
          callback_help: commands::delete::callback_help,
      },
      Command {
          name: "generate",
          callback_exec: commands::generate::callback_exec,
          callback_help: commands::generate::callback_help,
      },
      Command {
          name: "regenerate",
          callback_exec: commands::regenerate::callback_exec,
          callback_help: commands::regenerate::callback_help,
      },
      Command {
          name: "list",
          callback_exec: commands::list::callback_exec,
          callback_help: commands::list::callback_help,
      },
      Command {
          name: "export",
          callback_exec: commands::export::callback_exec,
          callback_help: commands::export::callback_help,
      },
      Command {
          name: "change-master-password",
          callback_exec: commands::change_master_password::callback_exec,
          callback_help: commands::change_master_password::callback_help,
      },
      Command {
          name: "rename",
          callback_exec: commands::rename::callback_exec,
          callback_help: commands::rename::callback_help,
      },
      Command {
          name: "change",
          callback_exec: commands::change::callback_exec,
          callback_help: commands::change::callback_help,
      },
      Command {
          name: "search",
          callback_exec: commands::search::callback_exec,
          callback_help: commands::search::callback_help,
      }];

fn command_from_name(name: &str) -> Option<&'static Command> {
    for c in COMMANDS.iter() {
        if c.name == name {
            return Some(c);
        }
    }
    None
}

fn open_password_file(filename: &str, create: bool) -> IoResult<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.write(true);
    options.create(create);
    options.open(&Path::new(filename))
}

fn get_password_file(filename: &str) -> IoResult<(Option<SafeString>, File)> {
    match open_password_file(filename, false) {
        Ok(file) => Ok((None, file)),
        Err(err) => {
            match err.kind() {
                IoErrorKind::NotFound => {
                    loop {
                        print_stderr!("I can't find your password file. \
                                                           Would you like to create one now \
                                                           (y/n)? ");
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line)?;
                        if line.starts_with('y') {
                            println_stderr!("");
                            println_stderr!("|----------------------------------------|");
                            println_stderr!("|                Awesome !               |");
                            println_stderr!("|----------------------------------------|");
                            println_stderr!("");
                            println_stderr!("In order to keep your passwords safe & \
                                             secure, we encrypt them using a Master \
                                             Password.");
                            println_stderr!("");
                            println_stderr!("The stronger it is, the better your passwords are \
                                             protected.");
                            println_stderr!("");

                            let master_password = prompt_password_stderr("What would you like \
                                                                          it to be? ").map(SafeString::new).map_err(|_| IoError::new(IoErrorKind::Other, FAIL_READING_NEW_PASSWORD))?;

                            let password_file = open_password_file(filename, true)?;

                            println_stderr!("");
                            println_stderr!("|----------------------------------------|");
                            println_stderr!("|           Running Rooster...           |");
                            println_stderr!("|----------------------------------------|");
                            println_stderr!("");

                            // std::env::args().collect()

                            return Ok((Some(master_password), password_file));
                        } else if line.starts_with('n') {
                            return Err(IoError::new(IoErrorKind::Other, DONT_CREATE_PASSWORD_FILE));
                        } else {
                            println_stderr!("I didn't get that. Should I create a password file \
                                             now (y/n)? ");
                        }
                    }
                }
                _ => Err(err),
            }
        }
    }
}

fn execute_command_from_filename(matches: &getopts::Matches,
                                 command: &Command,
                                 file: &mut File,
                                 master_password: SafeString)
                                 -> Result<(), i32> {

    let mut input: Vec<u8> = Vec::new();
    file.read_to_end(&mut input).map_err(|_| 1)?;

    // If the password file is empty (ie new), we'll make a new, empty store.
    let mut store = if input.is_empty() {
        password::v2::PasswordStore::new(master_password.clone()).map_err(|_| 1)?
    } else {
        // Try to open the file as is.
        match password::v2::PasswordStore::from_input(master_password.clone(),
                                                      SafeVec::new(input.clone())) {
            Ok(store) => store,
            Err(_) => {
                // If we can't open the file, we may need to upgrade its format first.
                match password::upgrade(master_password.clone(), SafeVec::new(input.clone())) {
                    Ok(store) => store,
                    Err(_) => {
                        // If we can't upgrade its format either, we show a helpful
                        // error message.
                        println_err!("I could not upgrade the Rooster file. This \
                                      could be because:");
                        println_err!("- you explicitly told Rooster not to open the \
                                      file,");
                        println_err!("- your version of Rooster is outdated,");
                        println_err!("- your Rooster file is corrupted,");
                        println_err!("- your master password is wrong.");
                        println_err!("Try upgrading to the latest version of Rooster.");
                        return Err(1);
                    }
                }
            }
        }
    };

    // Execute the command and save the new password list
    (command.callback_exec)(matches, &mut store)?;

    match store.sync(file) {
        Ok(()) => { Ok(()) }
        Err(err) => {
            println_err!("I could not save the password file (reason: {:?}).", err);
            Err(1)
        }
    }
}

fn get_password_file_path(rooster_file: Result<String, VarError>,
                          home_dir: Option<PathBuf>)
                          -> Result<String, i32> {
    match rooster_file {
        Ok(filename) => Ok(filename),
        Err(VarError::NotPresent) => {
            let mut filename = match home_dir {
                Some(home) => home.as_os_str().to_os_string().into_string().map_err(|_| 1)?,
                None => {
                    return Err(1);
                }
            };
            filename.push(PATH_SEP);
            filename.push_str(ROOSTER_FILE_DEFAULT);
            Ok(filename)
        }
        Err(VarError::NotUnicode(_)) => Err(1),
    }
}

fn ask_master_password() -> IoResult<SafeString> {
    prompt_password_stderr("Type your master password: ").map(SafeString::new)
}

fn usage(password_file: &str) {
    println!("Welcome to Rooster, the simple password manager for geeks :-)");
    println!("");
    println!("The current password file is: {}", password_file);
    println!("You may override this path in the $ROOSTER_FILE environment variable.");
    println!("");
    println!("Usage:");
    println!("    rooster -h");
    println!("    rooster [options] <command> [<args> ...]");
    println!("    rooster <command> -h");
    println!("");
    println!("Options:");
    println!("    -h, --help        Display a help message");
    println!("    -v, --version     Display the version of Rooster you are using");
    println!("    -a, --alnum       Only use alpha numeric (a-z, A-Z, 0-9) in generated passwords");
    println!("    -l, --length      Set a custom length for the generated password, default is 32");
    println!("    -s, --show        Show the password instead of copying it to the clipboard");
    println!("");
    println!("Commands:");
    println!("    add                        Add a new password manually");
    println!("    change                     Change a password manually");
    println!("    delete                     Delete a password");
    println!("    generate                   Generate a password");
    println!("    regenerate                 Re-generate a previously existing password");
    println!("    get                        Retrieve a password");
    println!("    rename                     Rename the app for a password");
    println!("    list                       List all apps and usernames");
    println!("    search                     Search for a specific password");
    println!("    export                     Dump all passwords in unencrypted JSON");
    println!("    change-master-password     Change your master password");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut opts = Options::new();
    opts.optflag("h", "help", "Display a help message");
    opts.optflag("v",
                 "version",
                 "Display the version of Rooster you are using");
    opts.optflag("a",
                 "alnum",
                 "Only use alpha numeric (a-z, A-Z, 0-9) in generated passwords");
    opts.optopt("l",
                "length",
                "Set a custom length for the generated password",
                "32");
    opts.optflag("s",
                 "show",
                 "Show the password instead of copying it to the clipboard");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(err) => {
            println_err!("{}", err);
            std::process::exit(1);
        }
    };

    // Fetch the Rooster file path now, so we can display it in help messages.
    let password_file_path = match get_password_file_path(env::var(ROOSTER_FILE_ENV_VAR),
                                                          env::home_dir()) {
        Ok(path) => path,
        Err(_) => {
            println_err!("Woops, I could not determine where your password file is.");
            println_err!("I recommend you try setting the $ROOSTER_FILE environment");
            println_err!("variable with the absolute path to your password file.");
            std::process::exit(1);
        }
    };

    // Global help was requested.
    if matches.opt_present("help") && matches.free.is_empty() {
        usage(password_file_path.deref());
        std::process::exit(0);
    }

    if matches.opt_present("version") {
        println!("v{}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    // No command was given, this is abnormal, so we'll show the docs.
    let command_name = match matches.free.get(0) {
        Some(command_name) => command_name,
        None => {
            usage(password_file_path.deref());
            std::process::exit(1);
        }
    };

    let command = match command_from_name(command_name.as_ref()) {
        Some(command) => command,
        None => {
            println_err!("Woops, the command `{}` does not exist. Try the --help option for more \
                          info.",
                         command_name);
            std::process::exit(1);
        }
    };

    if matches.opt_present("help") {
        (command.callback_help)();
        std::process::exit(0);
    }

    let (new_master_password, mut file) = match get_password_file(password_file_path.deref()) {
        Ok(file) => file,
        Err(err) => {
            if format!("{}", err) == DONT_CREATE_PASSWORD_FILE {
                println_err!("I can't go on without a password file, sorry");
            } else if format!("{}", err) == FAIL_READING_NEW_PASSWORD {
                println_err!("I couldn't read your Master Password, sorry");
            } else {
                println_err!("I can't find your password file at {} (reason: {})",
                             password_file_path,
                             err);
            }
            std::process::exit(1);
        }
    };

    let master_password = match new_master_password {
        Some(p) => p,
        None => {
            match ask_master_password() {
                Ok(p) => p,
                Err(err) => {
                    println_err!("Woops, I could not read your master password (reason: {}).",
                                 err);
                    std::process::exit(1);
                }
            }
        }
    };

    match execute_command_from_filename(&matches, command, &mut file, master_password) {
        Err(i) => std::process::exit(i),
        _ => std::process::exit(0),
    }
}
