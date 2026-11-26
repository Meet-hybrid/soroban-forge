use crate::ForgeError;

/// Returns the floor of the product divided by a positive denominator.
pub fn mul_div_floor(a: i128, b: i128, c: i128) -> Result<i128, ForgeError> {
    if a < 0 || b < 0 || c <= 0 {
        return Err(ForgeError::InvalidInput);
    }
    let (a, b, c) = (a as u128, b as u128, c as u128);
    let result = if let Some(product) = a.checked_mul(b) {
        product / c
    } else {
        let whole = (a / c)
            .checked_mul(b)
            .ok_or(ForgeError::ArithmeticOverflow)?;
        whole
            .checked_add(fractional_product(a % c, b, c))
            .ok_or(ForgeError::ArithmeticOverflow)?
    };
    i128::try_from(result).map_err(|_| ForgeError::ArithmeticOverflow)
}

fn fractional_product(a: u128, b: u128, c: u128) -> u128 {
    if let Some(product) = a.checked_mul(b) {
        return product / c;
    }

    // Process the bits of b from most to least significant, carrying the
    // quotient and remainder of a times the prefix processed so far.
    let mut quotient = 0;
    let mut remainder = 0;
    for bit in (0..(u128::BITS - b.leading_zeros())).rev() {
        let doubled = remainder * 2;
        quotient = quotient * 2 + doubled / c;
        remainder = doubled % c;
        if b & (1 << bit) != 0 {
            let sum = remainder + a;
            quotient += sum / c;
            remainder = sum % c;
        }
    }
    quotient
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_exact_and_floored_products() {
        let max = i128::MAX;
        for (a, b, c, expected) in [
            (0, max, 1, 0),
            (1, max, 1, max),
            (12, 5, 3, 20),
            (7, 5, 3, 11),
            (max, max, max, max),
            (max - 1, max - 1, max, max - 2),
            (max, 10_000, 10_000, max),
        ] {
            assert_eq!(mul_div_floor(a, b, c), Ok(expected));
        }
    }

    #[test]
    fn rejects_invalid_inputs_and_unrepresentable_results() {
        for (a, b, c) in [(-1, 2, 3), (1, -2, 3), (1, 2, 0), (1, 2, -3)] {
            assert_eq!(mul_div_floor(a, b, c), Err(ForgeError::InvalidInput));
        }
        assert_eq!(
            mul_div_floor(i128::MAX, i128::MAX, 1),
            Err(ForgeError::ArithmeticOverflow)
        );
    }
}
