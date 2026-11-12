//! `copy_snapshot_container`: a snapshot container streams to a sink, bounded and hashed
//! (personal-cfo-1cltt.5 slice C). The service is not trusted to stop, to be honest about length, or
//! to send the bytes the client recorded.

use std::io::{self, Cursor, Read, Write};

use sha2::{Digest, Sha256};
use sync_core::snapshot_service::{copy_snapshot_container, CopyError};

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A sink that records what it was given.
#[derive(Default)]
struct Recorder(Vec<u8>);

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A stream that yields `byte` forever: a service that never ends the response.
struct Endless(u8);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(self.0);
        Ok(buf.len())
    }
}

#[test]
fn the_bytes_arrive_whole_with_their_length_and_digest() {
    let bytes: Vec<u8> = (0..300_000_u32).map(|n| (n % 251) as u8).collect();
    let mut sink = Recorder::default();
    let copied = copy_snapshot_container(
        &mut Cursor::new(&bytes),
        &mut sink,
        bytes.len() as u64,
        Some(&sha(&bytes)),
    )
    .unwrap();
    assert_eq!(sink.0, bytes);
    assert_eq!(
        (copied.len, copied.sha256),
        (bytes.len() as u64, sha(&bytes))
    );
}

#[test]
fn a_container_exactly_at_the_bound_is_accepted_and_one_byte_over_is_not() {
    let bytes = vec![7_u8; 1000];
    let mut sink = Recorder::default();
    assert!(copy_snapshot_container(&mut Cursor::new(&bytes), &mut sink, 1000, None).is_ok());
    let mut sink = Recorder::default();
    let error =
        copy_snapshot_container(&mut Cursor::new(&bytes), &mut sink, 999, None).unwrap_err();
    assert!(matches!(error, CopyError::TooLarge { max_bytes: 999 }));
    assert!(sink.0.len() <= 999, "never more than the bound is written");
}

#[test]
fn a_service_that_never_stops_is_cut_off_at_the_bound() {
    let mut sink = Recorder::default();
    let error = copy_snapshot_container(&mut Endless(1), &mut sink, 200_000, None).unwrap_err();
    assert!(matches!(error, CopyError::TooLarge { max_bytes: 200_000 }));
    assert!(sink.0.len() <= 200_000);
}

#[test]
fn bytes_that_are_not_the_recorded_ones_are_a_digest_mismatch() {
    let bytes = vec![1_u8; 5000];
    let mut other = bytes.clone();
    other[4999] ^= 1;
    let mut sink = Recorder::default();
    let error = copy_snapshot_container(
        &mut Cursor::new(&bytes),
        &mut sink,
        10_000,
        Some(&sha(&other)),
    )
    .unwrap_err();
    assert!(matches!(error, CopyError::DigestMismatch));
}

#[test]
fn a_stream_or_a_sink_that_fails_is_an_io_error_and_an_interrupted_read_is_retried() {
    struct Fails(u8);
    impl Read for Fails {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;
            match self.0 {
                1 => Err(io::ErrorKind::Interrupted.into()),
                2 => {
                    buf[..4].copy_from_slice(b"DFSY");
                    Ok(4)
                }
                _ => Err(io::Error::other("connection reset")),
            }
        }
    }
    let mut sink = Recorder::default();
    let error = copy_snapshot_container(&mut Fails(0), &mut sink, 100, None).unwrap_err();
    assert!(matches!(error, CopyError::Io(_)), "{error:?}");
    assert_eq!(
        sink.0, b"DFSY",
        "the interrupted read was retried, not fatal"
    );

    struct Full;
    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let error = copy_snapshot_container(&mut Cursor::new(vec![0_u8; 10]), &mut Full, 100, None)
        .unwrap_err();
    assert!(matches!(error, CopyError::Io(_)));
}
