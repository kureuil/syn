// Copyright 2018 Syn Developers
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A punctuated sequence of syntax tree nodes separated by punctuation.
//!
//! Lots of things in Rust are punctuated sequences.
//!
//! - The fields of a struct are `Punctuated<Field, Token![,]>`.
//! - The segments of a path are `Punctuated<PathSegment, Token![::]>`.
//! - The bounds on a generic parameter are `Punctuated<TypeParamBound, Token![+]>`.
//! - The arguments to a function call are `Punctuated<Expr, Token![,]>`.
//!
//! This module provides a common representation for these punctuated sequences
//! in the form of the [`Punctuated<T, P>`] type. We store a vector of pairs of
//! syntax tree node + punctuation, where every node in the sequence is followed
//! by punctuation except for possibly the final one.
//!
//! [`Punctuated<T, P>`]: struct.Punctuated.html
//!
//! ```text
//! a_function_call(arg1, arg2, arg3);
//!                 ^^^^^ ~~~~~ ^^^^
//! ```

#[cfg(feature = "extra-traits")]
use std::fmt::{self, Debug};
#[cfg(any(feature = "full", feature = "derive"))]
use std::iter;
use std::iter::FromIterator;
use std::ops::{Index, IndexMut};
use std::option;
use std::slice;
use std::vec;

#[cfg(feature = "parsing")]
use buffer::Cursor;
#[cfg(feature = "parsing")]
use parse_error;
#[cfg(feature = "parsing")]
use synom::{PResult, Synom};

/// A punctuated sequence of syntax tree nodes of type `T` separated by
/// punctuation of type `P`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
#[cfg_attr(feature = "extra-traits", derive(Eq, PartialEq, Hash))]
#[cfg_attr(feature = "clone-impls", derive(Clone))]
pub struct Punctuated<T, P> {
    inner: Vec<(T, P)>,
    last: Option<Box<T>>,
}

impl<T, P> Punctuated<T, P> {
    /// Creates an empty punctuated sequence.
    pub fn new() -> Punctuated<T, P> {
        Punctuated {
            inner: Vec::new(),
            last: None,
        }
    }

    /// Determines whether this punctuated sequence is empty, meaning it
    /// contains no syntax tree nodes or punctuation.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0 && self.last.is_none()
    }

    /// Returns the number of syntax tree nodes in this punctuated sequence.
    ///
    /// This is the number of nodes of type `T`, not counting the punctuation of
    /// type `P`.
    pub fn len(&self) -> usize {
        self.inner.len() + if self.last.is_some() { 1 } else { 0 }
    }

    /// Borrows the first punctuated pair in this sequence.
    pub fn first(&self) -> Option<Pair<&T, &P>> {
        self.pairs().next()
    }

    /// Borrows the last punctuated pair in this sequence.
    pub fn last(&self) -> Option<Pair<&T, &P>> {
        if self.last.is_some() {
            self.last.as_ref().map(|t| Pair::End(t.as_ref()))
        } else {
            self.inner
                .last()
                .map(|&(ref t, ref d)| Pair::Punctuated(t, d))
        }
    }

    /// Mutably borrows the last punctuated pair in this sequence.
    pub fn last_mut(&mut self) -> Option<Pair<&mut T, &mut P>> {
        if self.last.is_some() {
            self.last.as_mut().map(|t| Pair::End(t.as_mut()))
        } else {
            self.inner
                .last_mut()
                .map(|&mut (ref mut t, ref mut d)| Pair::Punctuated(t, d))
        }
    }

    /// Returns an iterator over borrowed syntax tree nodes of type `&T`.
    pub fn iter(&self) -> Iter<T> {
        Iter {
            inner: Box::new(PrivateIter {
                inner: self.inner.iter(),
                last: self.last.as_ref().map(|t| t.as_ref()).into_iter(),
            }),
        }
    }

    /// Returns an iterator over mutably borrowed syntax tree nodes of type
    /// `&mut T`.
    pub fn iter_mut(&mut self) -> IterMut<T> {
        IterMut {
            inner: Box::new(PrivateIterMut {
                inner: self.inner.iter_mut(),
                last: self.last.as_mut().map(|t| t.as_mut()).into_iter(),
            }),
        }
    }

    /// Returns an iterator over the contents of this sequence as borrowed
    /// punctuated pairs.
    pub fn pairs(&self) -> Pairs<T, P> {
        Pairs {
            inner: self.inner.iter(),
            last: self.last.as_ref().map(|t| t.as_ref()).into_iter(),
        }
    }

    /// Returns an iterator over the contents of this sequence as mutably
    /// borrowed punctuated pairs.
    pub fn pairs_mut(&mut self) -> PairsMut<T, P> {
        PairsMut {
            inner: self.inner.iter_mut(),
            last: self.last.as_mut().map(|t| t.as_mut()).into_iter(),
        }
    }

    /// Returns an iterator over the contents of this sequence as owned
    /// punctuated pairs.
    pub fn into_pairs(self) -> IntoPairs<T, P> {
        IntoPairs {
            inner: self.inner.into_iter(),
            last: self.last.map(|t| *t).into_iter(),
        }
    }

    /// Appends a syntax tree node onto the end of this punctuated sequence. The
    /// sequence must previously have a trailing punctuation.
    ///
    /// Use [`push`] instead if the punctuated sequence may or may not already
    /// have trailing punctuation.
    ///
    /// [`push`]: #method.push
    ///
    /// # Panics
    ///
    /// Panics if the sequence does not already have a trailing punctuation when
    /// this method is called.
    pub fn push_value(&mut self, value: T) {
        assert!(self.empty_or_trailing());
        self.last = Some(Box::new(value));
    }

    /// Appends a trailing punctuation onto the end of this punctuated sequence.
    /// The sequence must be non-empty and must not already have trailing
    /// punctuation.
    ///
    /// # Panics
    ///
    /// Panics if the sequence is empty or already has a trailing punctuation.
    pub fn push_punct(&mut self, punctuation: P) {
        assert!(self.last.is_some());
        let last = self.last.take().unwrap();
        self.inner.push((*last, punctuation));
    }

    /// Removes the last punctuated pair from this sequence, or `None` if the
    /// sequence is empty.
    pub fn pop(&mut self) -> Option<Pair<T, P>> {
        if self.last.is_some() {
            self.last.take().map(|t| Pair::End(*t))
        } else {
            self.inner.pop().map(|(t, d)| Pair::Punctuated(t, d))
        }
    }

    /// Determines whether this punctuated sequence ends with a trailing
    /// punctuation.
    pub fn trailing_punct(&self) -> bool {
        self.last.is_none() && !self.is_empty()
    }

    /// Returns true if either this `Punctuated` is empty, or it has a trailing
    /// punctuation.
    ///
    /// Equivalent to `punctuated.is_empty() || punctuated.trailing_punct()`.
    pub fn empty_or_trailing(&self) -> bool {
        self.last.is_none()
    }
}

impl<T, P> Punctuated<T, P>
where
    P: Default,
{
    /// Appends a syntax tree node onto the end of this punctuated sequence.
    ///
    /// If there is not a trailing punctuation in this sequence when this method
    /// is called, the default value of punctuation type `P` is inserted before
    /// the given value of type `T`.
    pub fn push(&mut self, value: T) {
        if !self.empty_or_trailing() {
            self.push_punct(Default::default());
        }
        self.push_value(value);
    }

    /// Inserts an element at position `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is greater than the number of elements previously in
    /// this punctuated sequence.
    pub fn insert(&mut self, index: usize, value: T) {
        assert!(index <= self.len());

        if index == self.len() {
            self.push(value);
        } else {
            self.inner.insert(index, (value, Default::default()));
        }
    }
}

#[cfg(feature = "extra-traits")]
impl<T: Debug, P: Debug> Debug for Punctuated<T, P> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut list = f.debug_list();
        for &(ref t, ref p) in &self.inner {
            list.entry(t);
            list.entry(p);
        }
        if let Some(ref last) = self.last {
            list.entry(last);
        }
        list.finish()
    }
}

impl<T, P> FromIterator<T> for Punctuated<T, P>
where
    P: Default,
{
    fn from_iter<I: IntoIterator<Item = T>>(i: I) -> Self {
        let mut ret = Punctuated::new();
        ret.extend(i);
        ret
    }
}

impl<T, P> Extend<T> for Punctuated<T, P>
where
    P: Default,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, i: I) {
        for value in i {
            self.push(value);
        }
    }
}

impl<T, P> FromIterator<Pair<T, P>> for Punctuated<T, P> {
    fn from_iter<I: IntoIterator<Item = Pair<T, P>>>(i: I) -> Self {
        let mut ret = Punctuated::new();
        ret.extend(i);
        ret
    }
}

impl<T, P> Extend<Pair<T, P>> for Punctuated<T, P> {
    fn extend<I: IntoIterator<Item = Pair<T, P>>>(&mut self, i: I) {
        assert!(self.empty_or_trailing());
        let mut nomore = false;
        for pair in i {
            if nomore {
                panic!("Punctuated extended with items after a Pair::End");
            }
            match pair {
                Pair::Punctuated(a, b) => self.inner.push((a, b)),
                Pair::End(a) => {
                    self.last = Some(Box::new(a));
                    nomore = true;
                }
            }
        }
    }
}

impl<T, P> IntoIterator for Punctuated<T, P> {
    type Item = T;
    type IntoIter = IntoIter<T, P>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.inner.into_iter(),
            last: self.last.map(|t| *t).into_iter(),
        }
    }
}

impl<'a, T, P> IntoIterator for &'a Punctuated<T, P> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Punctuated::iter(self)
    }
}

impl<'a, T, P> IntoIterator for &'a mut Punctuated<T, P> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Punctuated::iter_mut(self)
    }
}

impl<T, P> Default for Punctuated<T, P> {
    fn default() -> Self {
        Punctuated::new()
    }
}

/// An iterator over borrowed pairs of type `Pair<&T, &P>`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct Pairs<'a, T: 'a, P: 'a> {
    inner: slice::Iter<'a, (T, P)>,
    last: option::IntoIter<&'a T>,
}

impl<'a, T, P> Iterator for Pairs<'a, T, P> {
    type Item = Pair<&'a T, &'a P>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|&(ref t, ref p)| Pair::Punctuated(t, p))
            .or_else(|| self.last.next().map(Pair::End))
    }
}

impl<'a, T, P> ExactSizeIterator for Pairs<'a, T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// An iterator over mutably borrowed pairs of type `Pair<&mut T, &mut P>`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct PairsMut<'a, T: 'a, P: 'a> {
    inner: slice::IterMut<'a, (T, P)>,
    last: option::IntoIter<&'a mut T>,
}

impl<'a, T, P> Iterator for PairsMut<'a, T, P> {
    type Item = Pair<&'a mut T, &'a mut P>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|&mut (ref mut t, ref mut p)| Pair::Punctuated(t, p))
            .or_else(|| self.last.next().map(Pair::End))
    }
}

impl<'a, T, P> ExactSizeIterator for PairsMut<'a, T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// An iterator over owned pairs of type `Pair<T, P>`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct IntoPairs<T, P> {
    inner: vec::IntoIter<(T, P)>,
    last: option::IntoIter<T>,
}

impl<T, P> Iterator for IntoPairs<T, P> {
    type Item = Pair<T, P>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(t, p)| Pair::Punctuated(t, p))
            .or_else(|| self.last.next().map(Pair::End))
    }
}

impl<T, P> ExactSizeIterator for IntoPairs<T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// An iterator over owned values of type `T`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct IntoIter<T, P> {
    inner: vec::IntoIter<(T, P)>,
    last: option::IntoIter<T>,
}

impl<T, P> Iterator for IntoIter<T, P> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|pair| pair.0)
            .or_else(|| self.last.next())
    }
}

impl<T, P> ExactSizeIterator for IntoIter<T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// An iterator over borrowed values of type `&T`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct Iter<'a, T: 'a> {
    inner: Box<ExactSizeIterator<Item = &'a T> + 'a>,
}

struct PrivateIter<'a, T: 'a, P: 'a> {
    inner: slice::Iter<'a, (T, P)>,
    last: option::IntoIter<&'a T>,
}

#[cfg(any(feature = "full", feature = "derive"))]
impl<'a, T> Iter<'a, T> {
    // Not public API.
    #[doc(hidden)]
    pub fn private_empty() -> Self {
        Iter {
            inner: Box::new(iter::empty()),
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'a, T> ExactSizeIterator for Iter<'a, T> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<'a, T, P> Iterator for PrivateIter<'a, T, P> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|pair| &pair.0)
            .or_else(|| self.last.next())
    }
}

impl<'a, T, P> ExactSizeIterator for PrivateIter<'a, T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// An iterator over mutably borrowed values of type `&mut T`.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub struct IterMut<'a, T: 'a> {
    inner: Box<ExactSizeIterator<Item = &'a mut T> + 'a>,
}

struct PrivateIterMut<'a, T: 'a, P: 'a> {
    inner: slice::IterMut<'a, (T, P)>,
    last: option::IntoIter<&'a mut T>,
}

#[cfg(any(feature = "full", feature = "derive"))]
impl<'a, T> IterMut<'a, T> {
    // Not public API.
    #[doc(hidden)]
    pub fn private_empty() -> Self {
        IterMut {
            inner: Box::new(iter::empty()),
        }
    }
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'a, T> ExactSizeIterator for IterMut<'a, T> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<'a, T, P> Iterator for PrivateIterMut<'a, T, P> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|pair| &mut pair.0)
            .or_else(|| self.last.next())
    }
}

impl<'a, T, P> ExactSizeIterator for PrivateIterMut<'a, T, P> {
    fn len(&self) -> usize {
        self.inner.len() + self.last.len()
    }
}

/// A single syntax tree node of type `T` followed by its trailing punctuation
/// of type `P` if any.
///
/// Refer to the [module documentation] for details about punctuated sequences.
///
/// [module documentation]: index.html
pub enum Pair<T, P> {
    Punctuated(T, P),
    End(T),
}

impl<T, P> Pair<T, P> {
    /// Extracts the syntax tree node from this punctuated pair, discarding the
    /// following punctuation.
    pub fn into_value(self) -> T {
        match self {
            Pair::Punctuated(t, _) | Pair::End(t) => t,
        }
    }

    /// Borrows the syntax tree node from this punctuated pair.
    pub fn value(&self) -> &T {
        match *self {
            Pair::Punctuated(ref t, _) | Pair::End(ref t) => t,
        }
    }

    /// Mutably borrows the syntax tree node from this punctuated pair.
    pub fn value_mut(&mut self) -> &mut T {
        match *self {
            Pair::Punctuated(ref mut t, _) | Pair::End(ref mut t) => t,
        }
    }

    /// Borrows the punctuation from this punctuated pair, unless this pair is
    /// the final one and there is no trailing punctuation.
    pub fn punct(&self) -> Option<&P> {
        match *self {
            Pair::Punctuated(_, ref d) => Some(d),
            Pair::End(_) => None,
        }
    }

    /// Creates a punctuated pair out of a syntax tree node and an optional
    /// following punctuation.
    pub fn new(t: T, d: Option<P>) -> Self {
        match d {
            Some(d) => Pair::Punctuated(t, d),
            None => Pair::End(t),
        }
    }

    /// Produces this punctuated pair as a tuple of syntax tree node and
    /// optional following punctuation.
    pub fn into_tuple(self) -> (T, Option<P>) {
        match self {
            Pair::Punctuated(t, d) => (t, Some(d)),
            Pair::End(t) => (t, None),
        }
    }
}

impl<T, P> Index<usize> for Punctuated<T, P> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        if index == self.len() - 1 {
            match self.last {
                Some(ref t) => t,
                None => &self.inner[index].0,
            }
        } else {
            &self.inner[index].0
        }
    }
}

impl<T, P> IndexMut<usize> for Punctuated<T, P> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        if index == self.len() - 1 {
            match self.last {
                Some(ref mut t) => t,
                None => &mut self.inner[index].0,
            }
        } else {
            &mut self.inner[index].0
        }
    }
}

#[cfg(feature = "parsing")]
impl<T, P> Punctuated<T, P>
where
    T: Synom,
    P: Synom,
{
    /// Parse **zero or more** syntax tree nodes with punctuation in between and
    /// **no trailing** punctuation.
    pub fn parse_separated(input: Cursor) -> PResult<Self> {
        Self::parse_separated_with(input, T::parse)
    }

    /// Parse **one or more** syntax tree nodes with punctuation in bewteen and
    /// **no trailing** punctuation.
    /// allowing trailing punctuation.
    pub fn parse_separated_nonempty(input: Cursor) -> PResult<Self> {
        Self::parse_separated_nonempty_with(input, T::parse)
    }

    /// Parse **zero or more** syntax tree nodes with punctuation in between and
    /// **optional trailing** punctuation.
    pub fn parse_terminated(input: Cursor) -> PResult<Self> {
        Self::parse_terminated_with(input, T::parse)
    }

    /// Parse **one or more** syntax tree nodes with punctuation in between and
    /// **optional trailing** punctuation.
    pub fn parse_terminated_nonempty(input: Cursor) -> PResult<Self> {
        Self::parse_terminated_nonempty_with(input, T::parse)
    }
}

#[cfg(feature = "parsing")]
impl<T, P> Punctuated<T, P>
where
    P: Synom,
{
    /// Parse **zero or more** syntax tree nodes using the given parser with
    /// punctuation in between and **no trailing** punctuation.
    pub fn parse_separated_with(input: Cursor, parse: fn(Cursor) -> PResult<T>) -> PResult<Self> {
        Self::parse(input, parse, false)
    }

    /// Parse **one or more** syntax tree nodes using the given parser with
    /// punctuation in between and **no trailing** punctuation.
    pub fn parse_separated_nonempty_with(
        input: Cursor,
        parse: fn(Cursor) -> PResult<T>,
    ) -> PResult<Self> {
        match Self::parse(input, parse, false) {
            Ok((ref b, _)) if b.is_empty() => parse_error(),
            other => other,
        }
    }

    /// Parse **zero or more** syntax tree nodes using the given parser with
    /// punctuation in between and **optional trailing** punctuation.
    pub fn parse_terminated_with(input: Cursor, parse: fn(Cursor) -> PResult<T>) -> PResult<Self> {
        Self::parse(input, parse, true)
    }

    /// Parse **one or more** syntax tree nodes using the given parser with
    /// punctuation in between and **optional trailing** punctuation.
    pub fn parse_terminated_nonempty_with(
        input: Cursor,
        parse: fn(Cursor) -> PResult<T>,
    ) -> PResult<Self> {
        match Self::parse(input, parse, true) {
            Ok((ref b, _)) if b.is_empty() => parse_error(),
            other => other,
        }
    }

    fn parse(
        mut input: Cursor,
        parse: fn(Cursor) -> PResult<T>,
        terminated: bool,
    ) -> PResult<Self> {
        let mut res = Punctuated::new();

        // get the first element
        match parse(input) {
            Err(_) => Ok((res, input)),
            Ok((o, i)) => {
                if i == input {
                    return parse_error();
                }
                input = i;
                res.push_value(o);

                // get the separator first
                while let Ok((s, i2)) = P::parse(input) {
                    if i2 == input {
                        break;
                    }

                    // get the element next
                    if let Ok((o3, i3)) = parse(i2) {
                        if i3 == i2 {
                            break;
                        }
                        res.push_punct(s);
                        res.push_value(o3);
                        input = i3;
                    } else {
                        break;
                    }
                }
                if terminated {
                    if let Ok((sep, after)) = P::parse(input) {
                        res.push_punct(sep);
                        input = after;
                    }
                }
                Ok((res, input))
            }
        }
    }
}

#[cfg(feature = "printing")]
mod printing {
    use super::*;
    use proc_macro2::TokenStream;
    use quote::{ToTokens, TokenStreamExt};

    impl<T, P> ToTokens for Punctuated<T, P>
    where
        T: ToTokens,
        P: ToTokens,
    {
        fn to_tokens(&self, tokens: &mut TokenStream) {
            tokens.append_all(self.pairs())
        }
    }

    impl<T, P> ToTokens for Pair<T, P>
    where
        T: ToTokens,
        P: ToTokens,
    {
        fn to_tokens(&self, tokens: &mut TokenStream) {
            match *self {
                Pair::Punctuated(ref a, ref b) => {
                    a.to_tokens(tokens);
                    b.to_tokens(tokens);
                }
                Pair::End(ref a) => a.to_tokens(tokens),
            }
        }
    }
}
