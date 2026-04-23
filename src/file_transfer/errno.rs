// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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
//
// SPDX-License-Identifier: Apache-2.0

use std::io;

pub(crate) const OK: i32 = 0;

// TODO use error number defined in libc or a wrapper library. Also check the correctness of the errors
pub(crate) fn to_errno(kind: io::ErrorKind) -> i32 {
    match kind {
        io::ErrorKind::NotFound => 2,                 // ENOENT
        io::ErrorKind::PermissionDenied => 13,        // EACCES
        io::ErrorKind::ConnectionRefused => 111,      // ECONNREFUSED
        io::ErrorKind::ConnectionReset => 104,        // ECONNRESET
        io::ErrorKind::HostUnreachable => 113,        // EHOSTUNREACH
        io::ErrorKind::NetworkUnreachable => 101,     // ENETUNREACH
        io::ErrorKind::ConnectionAborted => 103,      // ECONNABORTED
        io::ErrorKind::NotConnected => 107,           // ENOTCONN
        io::ErrorKind::AddrInUse => 98,               // EADDRINUSE
        io::ErrorKind::AddrNotAvailable => 99,        // EADDRNOTAVAIL
        io::ErrorKind::NetworkDown => 100,            // ENETDOWN
        io::ErrorKind::BrokenPipe => 32,              // EPIPE
        io::ErrorKind::AlreadyExists => 17,           // EEXIST
        io::ErrorKind::WouldBlock => 11,              // EWOULDBLOCK / EAGAIN
        io::ErrorKind::NotADirectory => 20,           // ENOTDIR
        io::ErrorKind::IsADirectory => 21,            // EISDIR
        io::ErrorKind::DirectoryNotEmpty => 39,       // ENOTEMPTY
        io::ErrorKind::ReadOnlyFilesystem => 30,      // EROFS
        io::ErrorKind::StaleNetworkFileHandle => 116, // ESTALE
        io::ErrorKind::InvalidInput => 22,            // EINVAL
        io::ErrorKind::InvalidData => 22,             // EINVAL (Rust specific, closest match)
        io::ErrorKind::TimedOut => 110,               // ETIMEDOUT
        io::ErrorKind::WriteZero => 28,               // ENOSPC (Closest approximation)
        io::ErrorKind::StorageFull => 28,             // ENOSPC
        io::ErrorKind::NotSeekable => 29,             // ESPIPE
        io::ErrorKind::QuotaExceeded => 122,          // EDQUOT
        io::ErrorKind::FileTooLarge => 27,            // EFBIG
        io::ErrorKind::ResourceBusy => 16,            // EBUSY
        io::ErrorKind::ExecutableFileBusy => 26,      // ETXTBSY
        io::ErrorKind::Deadlock => 35,                // EDEADLK
        io::ErrorKind::CrossesDevices => 18,          // EXDEV
        io::ErrorKind::TooManyLinks => 31,            // EMLINK
        io::ErrorKind::InvalidFilename => 36,         // ENAMETOOLONG
        io::ErrorKind::ArgumentListTooLong => 7,      // E2BIG
        io::ErrorKind::Interrupted => 4,              // EINTR
        io::ErrorKind::Unsupported => 95,             // ENOTSUP / EOPNOTSUPP
        io::ErrorKind::UnexpectedEof => 61,           // ENODATA (Closest approximation)
        io::ErrorKind::OutOfMemory => 12,             // ENOMEM
        io::ErrorKind::Other => 22,                   // EINVAL (Generic fallback)
        _ => 22,                                      // EINVAL (Generic fallback)
    }
}
