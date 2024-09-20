pub trait AlignExtend{
    fn is_power_of_two(&self) -> bool;
    fn align_up(&self, align: Self) -> Self;
    fn align_down(&self, align: Self) -> Self;
}

macro_rules! impl_align_extend{
    ($($uint_type:ty),+,) =>{
        $(
            impl AlignExtend for $uint_type{
                #[inline]
                fn is_power_of_two(&self) -> bool{
                    *self != 0 && *self & (*self - 1) == 0
                }

                #[inline]
                fn align_up(&self, align: Self) -> Self{
                    assert!(align.is_power_of_two() && align >= 2);
                    self.checked_add(align - 1).unwrap() & !(align - 1)
                }

                #[inline]
                fn align_down(&self, align: Self) -> Self{
                    assert!(align.is_power_of_two() && align >= 2);
                    self & !(align - 1)
                }
            }
        )*
    }
}

impl_align_extend!(u8, u16, u32, u64, u128,usize,);
