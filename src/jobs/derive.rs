// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod as_secs_opt {
    use std::time::Duration;

    use minicbor::Encode;
    use minicbor::decode::{self, Decoder};
    use minicbor::encode::{self, Encoder, Write};

    pub(crate) fn decode<'b, Ctx>(
        d: &mut Decoder<'b>,
        ctx: &mut Ctx,
    ) -> Result<Option<Duration>, decode::Error> {
        d.decode_with(ctx)
            .map(|secs: Option<u64>| secs.map(Duration::from_secs))
    }

    pub(crate) fn encode<Ctx, W: Write>(
        v: &Option<Duration>,
        e: &mut Encoder<W>,
        ctx: &mut Ctx,
    ) -> Result<(), encode::Error<W::Error>> {
        let secs: Option<u64> = v.map(|v| v.as_secs());

        secs.encode(e, ctx)
    }
}

pub(crate) mod url {
    use minicbor::Encode;
    use minicbor::decode::{self, Decoder};
    use minicbor::encode::{self, Encoder, Write};
    use url::Url;

    pub(crate) fn decode<'b, Ctx>(
        d: &mut Decoder<'b>,
        ctx: &mut Ctx,
    ) -> Result<Url, decode::Error> {
        d.decode_with(ctx)
            .and_then(|url| Url::parse(url).map_err(decode::Error::custom))
    }

    pub(crate) fn encode<Ctx, W: Write>(
        v: &Url,
        e: &mut Encoder<W>,
        ctx: &mut Ctx,
    ) -> Result<(), encode::Error<W::Error>> {
        v.as_str().encode(e, ctx)
    }
}

pub(crate) mod header_map {
    use std::str::FromStr;

    use minicbor::bytes::ByteSlice;
    use minicbor::decode::{self, Decoder};
    use minicbor::encode::{self, Encoder, Write};
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    pub(crate) fn decode<'b, Ctx>(
        d: &mut Decoder<'b>,
        ctx: &mut Ctx,
    ) -> Result<HeaderMap, decode::Error> {
        d.map_iter_with::<Ctx, &str, &ByteSlice>(ctx)?
            .map(|res| {
                let (key, value) = res?;

                let key = HeaderName::from_str(key).map_err(decode::Error::custom)?;
                let value =
                    HeaderValue::from_bytes(value.as_ref()).map_err(decode::Error::custom)?;

                Ok((key, value))
            })
            .collect::<Result<HeaderMap, decode::Error>>()
    }

    pub(crate) fn encode<Ctx, W: Write>(
        v: &HeaderMap,
        e: &mut Encoder<W>,
        ctx: &mut Ctx,
    ) -> Result<(), encode::Error<W::Error>> {
        let len = u64::try_from(v.len()).map_err(encode::Error::custom)?;

        e.map(len)?;

        for (k, v) in v {
            let bytes: &ByteSlice = v.as_bytes().into();

            e.encode_with(k.as_str(), ctx)?.encode_with(bytes, ctx)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ::url::Url;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use rstest::{Context, fixture, rstest};
    use std::time::Duration;

    use crate::tests::{Hexdump, with_insta};

    #[derive(Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
    struct TestCbor {
        #[cbor(n(0), with = "super::as_secs_opt")]
        opt_duration: Option<Duration>,
        #[cbor(n(1), with = "super::url")]
        url: Url,
        #[cbor(n(2), with = "super::header_map")]
        header_map: HeaderMap,
    }

    #[fixture]
    fn mk_cbor() -> TestCbor {
        TestCbor {
            opt_duration: Some(Duration::from_secs(42)),
            url: "https://www.example.com/path?query=1".parse().unwrap(),
            header_map: [
                (
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static("application/cbor"),
                ),
                (
                    HeaderName::from_static("content-length"),
                    HeaderValue::from_static("42"),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[fixture]
    fn mk_cbor_nones() -> TestCbor {
        TestCbor {
            opt_duration: None,
            url: "https://www.example.com/path?query=1".parse().unwrap(),
            header_map: [
                (
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static("application/cbor"),
                ),
                (
                    HeaderName::from_static("content-length"),
                    HeaderValue::from_static("42"),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[rstest]
    #[case(mk_cbor())]
    #[case(mk_cbor_nones())]
    fn cbor_roundtrip(#[case] value: TestCbor, #[context] ctx: Context) {
        let buf = minicbor::to_vec(&value).unwrap();
        let res: TestCbor = minicbor::decode(&buf).unwrap();

        assert_eq!(res, value);

        with_insta!({
            let name = ctx.case.unwrap().to_string();
            insta::assert_snapshot!(name, Hexdump(buf));
        });
    }
}
