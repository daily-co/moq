use bytes::Bytes;

use super::{BoundsExceeded, Form, VarInt};

/// An error that occurs during encoding.
#[derive(thiserror::Error, Debug, Clone)]
#[non_exhaustive]
pub enum EncodeError {
	/// An integer was too large for the QUIC varint range.
	#[error("bounds exceeded")]
	BoundsExceeded,
	/// The payload exceeds the maximum size the wire format can express.
	#[error("too large")]
	TooLarge,
	/// The destination buffer had no room for the value.
	#[error("short buffer")]
	Short,
	/// The message cannot be encoded from the current session state.
	#[error("invalid state")]
	InvalidState,
	/// A repeated field exceeded the count the wire format permits.
	#[error("too many")]
	TooMany,
	/// The field does not exist in the negotiated protocol version.
	#[error("unsupported version")]
	Version,
	/// The value is well-formed but this implementation cannot put it on the wire.
	#[error("unsupported")]
	Unsupported,
}

impl From<BoundsExceeded> for EncodeError {
	fn from(_: BoundsExceeded) -> Self {
		Self::BoundsExceeded
	}
}

/// Write the value to an [`Encoder`] using the given version.
pub trait Encode<V> {
	/// Encode the value to the given encoder.
	fn encode(&self, w: &mut Encoder<'_>, version: V) -> Result<(), EncodeError>;

	/// Encode the value into a fresh [Bytes] buffer.
	fn encode_bytes(&self, version: V) -> Result<Bytes, EncodeError>
	where
		V: Into<Form> + Copy,
	{
		let mut buf = Vec::new();
		self.encode(&mut Encoder::new(&mut buf, version.into()), version)?;
		Ok(buf.into())
	}
}

/// Appends wire primitives to a byte buffer.
///
/// The buffer grows as needed, so only a value the wire cannot express fails.
#[derive(Debug)]
pub struct Encoder<'a> {
	buf: &'a mut Vec<u8>,
	form: Form,
}

impl<'a> Encoder<'a> {
	/// Append to `buf`, with varints in the given form.
	pub fn new(buf: &'a mut Vec<u8>, form: Form) -> Self {
		Self { buf, form }
	}

	/// The varint form this encoder writes.
	pub fn form(&self) -> Form {
		self.form
	}

	/// Where the next byte goes: the buffer's length, including any written before this
	/// encoder. Mark a size-prefixed body's start with it.
	pub fn position(&self) -> usize {
		self.buf.len()
	}

	/// Write raw bytes.
	pub fn slice(&mut self, v: &[u8]) {
		self.buf.extend_from_slice(v);
	}

	/// Write a single byte.
	pub fn u8(&mut self, v: u8) {
		self.buf.push(v);
	}

	/// Write a big-endian `u16`.
	pub fn u16(&mut self, v: u16) {
		self.buf.extend_from_slice(&v.to_be_bytes());
	}

	/// Write a bool as a 0 or 1 byte.
	pub fn bool(&mut self, v: bool) {
		self.buf.push(v as u8);
	}

	/// Write a varint, or fail with [`EncodeError::BoundsExceeded`] if the form cannot carry it.
	pub fn varint(&mut self, v: VarInt) -> Result<(), EncodeError> {
		let (buf, len) = v.encode_form(self.form)?;
		self.buf.extend_from_slice(&buf[..len]);
		Ok(())
	}

	/// Write an optional varint: `None` as 0, and `Some(n)` as `n + 1`.
	pub fn varint_opt(&mut self, v: Option<u64>) -> Result<(), EncodeError> {
		let v = match v {
			Some(v) => v.checked_add(1).ok_or(EncodeError::TooLarge)?,
			None => 0,
		};
		self.varint(v.into())
	}

	/// Write a varint length, then the raw bytes.
	pub fn bytes(&mut self, v: &[u8]) -> Result<(), EncodeError> {
		self.varint(v.len().into())?;
		self.slice(v);
		Ok(())
	}

	/// Write a varint length, then the UTF-8 bytes.
	pub fn string(&mut self, v: &str) -> Result<(), EncodeError> {
		self.bytes(v.as_bytes())
	}

	/// Prefix everything written since [`Self::position`] was `start` with its varint length.
	pub fn prefix_varint(&mut self, start: usize) -> Result<(), EncodeError> {
		let size = VarInt::from(self.buf.len() - start);
		let (prefix, len) = size.encode_form(self.form)?;
		self.buf.splice(start..start, prefix[..len].iter().copied());
		Ok(())
	}

	/// Prefix everything written since [`Self::position`] was `start` with its `u16` length.
	pub fn prefix_u16(&mut self, start: usize) -> Result<(), EncodeError> {
		let size = u16::try_from(self.buf.len() - start).map_err(|_| EncodeError::TooLarge)?;
		self.buf.splice(start..start, size.to_be_bytes());
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The prefix lands before the body, sized to it, even when it takes more than a byte.
	#[test]
	fn prefix_varint_sizes_the_body() {
		for size in [0usize, 63, 64, 20_000] {
			let mut buf = vec![0xaa];
			let mut w = Encoder::new(&mut buf, Form::Quic);
			let start = w.position();
			w.slice(&vec![0x55; size]);
			w.prefix_varint(start).unwrap();

			let mut r = super::super::Decoder::new(&buf[1..], Form::Quic);
			assert_eq!(buf[0], 0xaa);
			assert_eq!(r.varint().unwrap().into_inner(), size as u64);
			assert_eq!(r.rest(), vec![0x55; size]);
		}
	}
}
