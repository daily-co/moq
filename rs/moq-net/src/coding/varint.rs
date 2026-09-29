// Based on quinn-proto
// https://github.com/quinn-rs/quinn/blob/main/quinn-proto/src/varint.rs
// Licensed via Apache 2.0 and MIT

use std::fmt;

use thiserror::Error;

use super::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder};
use crate::{Version, ietf, lite};

/// The number does not fit the target: a varint wire form or a narrower integer.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Error)]
#[error("value out of range")]
pub struct BoundsExceeded;

/// An integer destined for the wire as a variable-length integer.
///
/// It holds the full `u64` range, which the leading-ones form of moq-transport draft-17+
/// can carry. The QUIC form (moq-lite, drafts 14-16) tops out at `2^62 - 1`, so encoding
/// a larger value there fails with [`BoundsExceeded`] rather than truncating.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VarInt(u64);

impl VarInt {
	/// The largest possible value.
	pub const MAX: Self = Self(u64::MAX);

	/// The largest value the QUIC form can carry: `2^62 - 1`.
	pub const MAX_QUIC: Self = Self((1 << 62) - 1);

	/// The smallest possible value.
	pub const ZERO: Self = Self(0);

	/// Construct from a `u32`, usable in `const` contexts.
	pub const fn from_u32(x: u32) -> Self {
		Self(x as u64)
	}

	/// Construct from a `u64`, usable in `const` contexts.
	pub const fn from_u64(x: u64) -> Self {
		Self(x)
	}

	/// Construct from a `u128`, or `None` if it exceeds [`Self::MAX`].
	pub const fn from_u128(x: u128) -> Option<Self> {
		if x <= u64::MAX as u128 {
			Some(Self(x as u64))
		} else {
			None
		}
	}

	/// Extract the integer value
	pub const fn into_inner(self) -> u64 {
		self.0
	}

	/// Map a signed `i64` onto the unsigned range with zigzag: `(n << 1) ^ (n >> 63)`.
	///
	/// Small negative numbers map to small unsigneds (-1 -> 1, 1 -> 2, -2 -> 3, ...), and
	/// the whole `i64` range fits.
	pub const fn from_zigzag(signed: i64) -> Self {
		Self(((signed << 1) ^ (signed >> 63)) as u64)
	}

	/// Decode this varint as a signed `i64` via the inverse zigzag transform.
	pub const fn to_zigzag(self) -> i64 {
		let v = self.0;
		((v >> 1) as i64) ^ -((v & 1) as i64)
	}
}

impl From<VarInt> for u64 {
	fn from(x: VarInt) -> Self {
		x.0
	}
}

impl From<VarInt> for u128 {
	fn from(x: VarInt) -> Self {
		x.0 as u128
	}
}

impl From<u8> for VarInt {
	fn from(x: u8) -> Self {
		Self(x.into())
	}
}

impl From<u16> for VarInt {
	fn from(x: u16) -> Self {
		Self(x.into())
	}
}

impl From<u32> for VarInt {
	fn from(x: u32) -> Self {
		Self(x.into())
	}
}

impl From<u64> for VarInt {
	fn from(x: u64) -> Self {
		Self(x)
	}
}

impl From<usize> for VarInt {
	fn from(x: usize) -> Self {
		// usize is at most 64 bits on every target Rust supports.
		Self(x as u64)
	}
}

impl TryFrom<u128> for VarInt {
	type Error = BoundsExceeded;

	/// Succeeds iff `x` < 2^64
	fn try_from(x: u128) -> Result<Self, BoundsExceeded> {
		Self::from_u128(x).ok_or(BoundsExceeded)
	}
}

impl TryFrom<VarInt> for usize {
	type Error = BoundsExceeded;

	/// Succeeds iff `x` fits the target's pointer width.
	fn try_from(x: VarInt) -> Result<Self, BoundsExceeded> {
		usize::try_from(x.0).map_err(|_| BoundsExceeded)
	}
}

impl TryFrom<VarInt> for u32 {
	type Error = BoundsExceeded;

	/// Succeeds iff `x` < 2^32
	fn try_from(x: VarInt) -> Result<Self, BoundsExceeded> {
		u32::try_from(x.0).map_err(|_| BoundsExceeded)
	}
}

impl TryFrom<VarInt> for u16 {
	type Error = BoundsExceeded;

	/// Succeeds iff `x` < 2^16
	fn try_from(x: VarInt) -> Result<Self, BoundsExceeded> {
		u16::try_from(x.0).map_err(|_| BoundsExceeded)
	}
}

impl TryFrom<VarInt> for u8 {
	type Error = BoundsExceeded;

	/// Succeeds iff `x` < 2^8
	fn try_from(x: VarInt) -> Result<Self, BoundsExceeded> {
		u8::try_from(x.0).map_err(|_| BoundsExceeded)
	}
}

impl fmt::Display for VarInt {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

/// How a protocol version lays out a [`VarInt`] on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
	/// QUIC's two-bit length tag, up to [`VarInt::MAX_QUIC`].
	Quic,
	/// Leading one bits count the length, up to [`VarInt::MAX`].
	LeadingOnes {
		/// Whether the 7-byte form (`1111110x`) is accepted on decode, which draft-17 forbids.
		seven: bool,
	},
}

impl From<lite::Version> for Form {
	fn from(version: lite::Version) -> Self {
		match version {
			lite::Version::Lite01
			| lite::Version::Lite02
			| lite::Version::Lite03
			| lite::Version::Lite04
			| lite::Version::Lite05
			| lite::Version::Lite06
			| lite::Version::Lite07 => Self::Quic,
		}
	}
}

impl From<ietf::Version> for Form {
	fn from(version: ietf::Version) -> Self {
		match version {
			ietf::Version::Draft14 | ietf::Version::Draft15 | ietf::Version::Draft16 => Self::Quic,
			ietf::Version::Draft17 => Self::LeadingOnes { seven: false },
			_ => Self::LeadingOnes { seven: true },
		}
	}
}

impl From<Version> for Form {
	fn from(version: Version) -> Self {
		match version {
			Version::Lite(v) => v.into(),
			Version::Ietf(v) => v.into(),
		}
	}
}

impl VarInt {
	/// The high and low 32 bits.
	///
	/// The codec works on the halves so it never needs 64-bit bitwise math, which a
	/// JavaScript `number` cannot do.
	const fn to_halves(self) -> (u32, u32) {
		((self.0 >> 32) as u32, self.0 as u32)
	}

	/// The inverse of [`Self::to_halves`].
	const fn from_halves(hi: u32, lo: u32) -> Self {
		Self(((hi as u64) << 32) | lo as u64)
	}

	/// Decode from the front of `buf`, returning the value and the bytes it took.
	pub(super) fn decode_form(buf: &[u8], form: Form) -> Result<(Self, usize), DecodeError> {
		let Some(&first) = buf.first() else {
			return Err(DecodeError::Short);
		};

		let (len, head) = match form {
			Form::Quic => (1usize << (first >> 6), first & 0x3f),
			Form::LeadingOnes { seven } => {
				let ones = first.leading_ones();
				if ones == 6 && !seven {
					return Err(DecodeError::InvalidValue);
				}
				// `0x7f >> 8` would overflow, and there are no value bits left anyway.
				let head = if ones >= 7 { 0 } else { first & (0x7f >> ones) };
				(ones as usize + 1, head)
			}
		};

		let Some(rest) = buf.get(1..len) else {
			return Err(DecodeError::Short);
		};

		let mut hi = 0u32;
		let mut lo = head as u32;
		for &byte in rest {
			hi = (hi << 8) | (lo >> 24);
			lo = (lo << 8) | byte as u32;
		}

		Ok((Self::from_halves(hi, lo), len))
	}

	/// The minimal encoding in the given form: the bytes, and how many of them are used.
	///
	/// Fails past [`Self::MAX_QUIC`] in the QUIC form.
	pub(super) fn encode_form(self, form: Form) -> Result<([u8; 9], usize), BoundsExceeded> {
		let (hi, lo) = self.to_halves();
		let [a, b, c, d] = lo.to_be_bytes();
		let [e, f, g, h] = hi.to_be_bytes();

		Ok(match form {
			Form::Quic if hi == 0 && lo < 1 << 6 => ([d, 0, 0, 0, 0, 0, 0, 0, 0], 1),
			Form::Quic if hi == 0 && lo < 1 << 14 => ([0x40 | c, d, 0, 0, 0, 0, 0, 0, 0], 2),
			Form::Quic if hi == 0 && lo < 1 << 30 => ([0x80 | a, b, c, d, 0, 0, 0, 0, 0], 4),
			Form::Quic if hi < 1 << 30 => ([0xc0 | e, f, g, h, a, b, c, d, 0], 8),
			Form::Quic => return Err(BoundsExceeded),
			Form::LeadingOnes { .. } if hi == 0 && lo < 1 << 7 => ([d, 0, 0, 0, 0, 0, 0, 0, 0], 1),
			Form::LeadingOnes { .. } if hi == 0 && lo < 1 << 14 => ([0x80 | c, d, 0, 0, 0, 0, 0, 0, 0], 2),
			Form::LeadingOnes { .. } if hi == 0 && lo < 1 << 21 => ([0xc0 | b, c, d, 0, 0, 0, 0, 0, 0], 3),
			Form::LeadingOnes { .. } if hi == 0 && lo < 1 << 28 => ([0xe0 | a, b, c, d, 0, 0, 0, 0, 0], 4),
			Form::LeadingOnes { .. } if hi < 1 << 3 => ([0xf0 | h, a, b, c, d, 0, 0, 0, 0], 5),
			Form::LeadingOnes { .. } if hi < 1 << 10 => ([0xf8 | g, h, a, b, c, d, 0, 0, 0], 6),
			// The 7-byte form is skipped: one byte longer, but legal on every draft.
			Form::LeadingOnes { .. } if hi < 1 << 24 => ([0xfe, f, g, h, a, b, c, d, 0], 8),
			Form::LeadingOnes { .. } => ([0xff, e, f, g, h, a, b, c, d], 9),
		})
	}

	/// The bytes this takes on the wire in the given form, or [`BoundsExceeded`] if it
	/// does not fit.
	pub(crate) fn size(self, form: Form) -> Result<usize, BoundsExceeded> {
		Ok(self.encode_form(form)?.1)
	}

	/// Decode a QUIC-style varint (2-bit length tag in top bits).
	pub fn decode_quic<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
		let Some(&first) = r.chunk().first() else {
			return Err(DecodeError::Short);
		};

		// Copy out so a varint split across chunks still decodes.
		let len = 1usize << (first >> 6);
		if r.remaining() < len {
			return Err(DecodeError::Short);
		}
		let mut buf = [0u8; 8];
		r.copy_to_slice(&mut buf[..len]);

		Ok(Self::decode_form(&buf[..len], Form::Quic)?.0)
	}

	/// Encode a QUIC-style varint (2-bit length tag in top bits).
	///
	/// Fails with [`EncodeError::BoundsExceeded`] past [`Self::MAX_QUIC`].
	pub fn encode_quic<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
		let (buf, len) = self.encode_form(Form::Quic)?;
		if w.remaining_mut() < len {
			return Err(EncodeError::Short);
		}
		w.put_slice(&buf[..len]);
		Ok(())
	}
}

impl<V> Encode<V> for VarInt {
	fn encode(&self, w: &mut Encoder<'_>, _: V) -> Result<(), EncodeError> {
		w.varint(*self)
	}
}

impl<V> Decode<V> for VarInt {
	fn decode(r: &mut Decoder<'_>, _: V) -> Result<Self, DecodeError> {
		r.varint()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const DRAFT17: Form = Form::LeadingOnes { seven: false };
	const DRAFT18: Form = Form::LeadingOnes { seven: true };

	fn encode(value: VarInt, form: Form) -> Result<Vec<u8>, BoundsExceeded> {
		let (buf, len) = value.encode_form(form)?;
		Ok(buf[..len].to_vec())
	}

	/// Test vectors from the draft-17 spec (Table 2: Example Integer Encodings),
	/// excluding the known-buggy example 4 (0xdd7f3e7d).
	#[test]
	fn leading_ones_spec_examples() {
		let cases: &[(&[u8], u64)] = &[
			(&[0x25], 37),
			(&[0x80, 0x25], 37),
			(&[0xbb, 0xbd], 15_293),
			// Example 4 (0xdd7f3e7d = 494,878,333) is omitted. The spec has a bug.
			// See https://github.com/moq-wg/moq-transport/pull/1521
			(&[0xfa, 0xa1, 0xa0, 0xe4, 0x03, 0xd8], 2_893_212_287_960),
			(
				&[0xfe, 0xfa, 0x31, 0x8f, 0xa8, 0xe3, 0xca, 0x11],
				70_423_237_261_249_041,
			),
			(
				&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
				18_446_744_073_709_551_615,
			),
		];

		for (bytes, expected) in cases {
			let (decoded, len) = VarInt::decode_form(bytes, DRAFT17).expect("decode should succeed");
			assert_eq!(
				decoded.into_inner(),
				*expected,
				"decode mismatch for bytes {bytes:02x?}"
			);
			assert_eq!(len, bytes.len(), "all bytes should be consumed for {bytes:02x?}");

			// Skip the non-minimal encoding (0x8025 for 37); we only emit the minimal one.
			if bytes.len() == 1 || *expected != 37 {
				let encoded = encode(VarInt::from(*expected), DRAFT17).unwrap();
				assert_eq!(&encoded, bytes, "encode mismatch for value {expected}");
			}
		}
	}

	/// 11111100 (0xFC) is an invalid code point on draft-17 (allowed as 7-byte form on draft-18+).
	#[test]
	fn leading_ones_invalid_0xfc() {
		assert!(
			matches!(VarInt::decode_form(&[0xFC], DRAFT17), Err(DecodeError::InvalidValue)),
			"0xFC should be rejected as invalid on draft-17"
		);
	}

	#[test]
	fn leading_ones_boundaries_round_trip() {
		let cases = [
			((1u64 << 7) - 1, 1usize),
			(1u64 << 7, 2usize),
			((1u64 << 14) - 1, 2usize),
			(1u64 << 14, 3usize),
			((1u64 << 56) - 1, 8usize),
			(1u64 << 56, 9usize),
		];

		for (value, expected_len) in cases {
			let encoded = encode(VarInt::from(value), DRAFT17).unwrap();
			assert_eq!(
				encoded.len(),
				expected_len,
				"unexpected encoded length for value {value}"
			);

			let (decoded, _) = VarInt::decode_form(&encoded, DRAFT17).expect("leading-ones decode should succeed");
			assert_eq!(decoded.into_inner(), value, "round-trip mismatch for value {value}");
		}
	}

	/// Every length class of both forms survives a round trip, including the boundaries
	/// of each class.
	#[test]
	fn every_length_round_trips() {
		for bits in 0..64 {
			for value in [1u64 << bits, (1u64 << bits) - 1, (1u64 << bits) + 1] {
				let value = VarInt::from(value);
				for form in [Form::Quic, DRAFT17, DRAFT18] {
					let Ok(encoded) = encode(value, form) else {
						assert_eq!(form, Form::Quic);
						assert!(value > VarInt::MAX_QUIC);
						continue;
					};
					let (decoded, len) = VarInt::decode_form(&encoded, form).unwrap();
					assert_eq!((decoded, len), (value, encoded.len()), "{form:?} {value}");
				}
			}
		}
	}

	/// The QUIC form stops at 2^62 - 1 and refuses anything past it rather than
	/// truncating, while the leading-ones form carries the whole u64.
	#[test]
	fn quic_refuses_past_62_bits() {
		let max = VarInt::MAX_QUIC;
		assert_eq!(max.into_inner(), (1 << 62) - 1);
		assert_eq!(encode(max, Form::Quic).unwrap(), [0xff; 8]);

		for value in [1u64 << 62, u64::MAX] {
			assert_eq!(encode(VarInt::from(value), Form::Quic), Err(BoundsExceeded));
			assert!(matches!(
				VarInt::from(value).encode_quic(&mut Vec::new()),
				Err(EncodeError::BoundsExceeded)
			));
		}

		for value in [(1u64 << 62) - 1, 1u64 << 62, u64::MAX] {
			let encoded = encode(VarInt::from(value), DRAFT18).unwrap();
			assert_eq!(encoded.len(), 9);
			assert_eq!(VarInt::decode_form(&encoded, DRAFT18).unwrap().0.into_inner(), value);
		}
	}

	#[test]
	fn draft17_rejects_7_byte_varint() {
		// 1111110x prefix: invalid on draft-17.
		let err = VarInt::decode_form(&[0xFC, 0, 0, 0, 0, 0, 0], DRAFT17).unwrap_err();
		assert!(matches!(err, DecodeError::InvalidValue));
	}

	#[test]
	fn zigzag_roundtrip_small() {
		for n in [-3i64, -2, -1, 0, 1, 2, 3, 100, -100] {
			let v = VarInt::from_zigzag(n);
			assert_eq!(v.to_zigzag(), n, "roundtrip failed for {}", n);
		}
	}

	#[test]
	fn zigzag_small_values_compact() {
		// First few values should fit in 1 byte (varint range 0..=63 = top-2-bits tag 00).
		assert_eq!(VarInt::from_zigzag(0).into_inner(), 0);
		assert_eq!(VarInt::from_zigzag(-1).into_inner(), 1);
		assert_eq!(VarInt::from_zigzag(1).into_inner(), 2);
		assert_eq!(VarInt::from_zigzag(-2).into_inner(), 3);
		assert_eq!(VarInt::from_zigzag(2).into_inner(), 4);
	}

	/// Zigzag covers the whole i64 range; only the QUIC form bounds what goes on the wire.
	#[test]
	fn zigzag_roundtrip_boundary() {
		let mid = (1i64 << 30) + 17;

		for n in [i64::MAX, i64::MIN, (1i64 << 61) - 1, -(1i64 << 61), mid, -mid] {
			let v = VarInt::from_zigzag(n);
			assert_eq!(v.to_zigzag(), n);
		}

		assert_eq!(VarInt::from_zigzag(i64::MIN), VarInt::MAX);
		assert!(VarInt::from_zigzag(1i64 << 61) > VarInt::MAX_QUIC);
		assert_eq!(VarInt::from_zigzag(-(1i64 << 61)), VarInt::MAX_QUIC);
	}

	#[test]
	fn zigzag_quic_varint_roundtrip() {
		// Encode a zigzag value through the QUIC varint wire format.
		for n in [-5000i64, 0, 100, -1, 1_000_000, -1_000_000] {
			let v = VarInt::from_zigzag(n);
			let bytes = v.encode_bytes(lite::Version::Lite01).unwrap();
			let (decoded, _) = VarInt::decode_slice(&bytes, lite::Version::Lite01).unwrap();
			assert_eq!(decoded.to_zigzag(), n);
		}
	}

	#[test]
	fn draft18_accepts_7_byte_varint() {
		// Value 0x1234_5678_9ABC encoded as 7-byte leading-ones (1111110x | hi, +6 bytes).
		let value: u64 = 0x1234_5678_9ABC;
		let mut bytes = Vec::new();
		// Prefix byte: 1111110_0 + (value >> 48) bit. Top 1 bit of 49 = bit 48.
		// value fits in 49 bits, so the 0x01 LSB of prefix encodes bit 48 of value.
		let hi_bit = ((value >> 48) & 0x01) as u8;
		bytes.push(0xFC | hi_bit);
		for shift in (0..48).step_by(8).rev() {
			bytes.push(((value >> shift) & 0xFF) as u8);
		}
		let (decoded, _) = VarInt::decode_form(&bytes, DRAFT18).unwrap();
		assert_eq!(decoded.into_inner(), value);
	}

	/// The Buf-based helpers other crates use read and write the same bytes, even when
	/// the varint straddles two chunks.
	#[test]
	fn quic_helpers_match_the_codec() {
		use bytes::Buf;

		let value = VarInt::from(0x1234_5678u64);
		let mut out = Vec::new();
		value.encode_quic(&mut out).unwrap();
		assert_eq!(out, encode(value, Form::Quic).unwrap());

		let mut split = (&out[..2]).chain(&out[2..]);
		assert_eq!(VarInt::decode_quic(&mut split).unwrap(), value);
		assert!(!split.has_remaining());
	}
}
