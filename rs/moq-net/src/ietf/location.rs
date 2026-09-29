use crate::coding::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder, VarInt};

use super::Version;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
	pub group: u64,
	pub object: u64,
}

impl Encode<Version> for Location {
	fn encode(&self, w: &mut Encoder<'_>, _: Version) -> Result<(), EncodeError> {
		w.varint(VarInt::from(self.group))?;
		w.varint(VarInt::from(self.object))?;
		Ok(())
	}
}

impl Decode<Version> for Location {
	fn decode(buf: &mut Decoder<'_>, _: Version) -> Result<Self, DecodeError> {
		let group = buf.varint()?.into_inner();
		let object = buf.varint()?.into_inner();
		Ok(Self { group, object })
	}
}
