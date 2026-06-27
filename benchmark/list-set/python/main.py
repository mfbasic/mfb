"""Fixed-width in-place set: build a 200-element list, then run 10 passes
incrementing every element. Prints the checksum (sum of all elements)."""


def main():
    nums = list(range(200))
    for _pass in range(10):
        for j in range(200):
            nums[j] = nums[j] + 1
    checksum = sum(nums)
    print("checksum: " + str(checksum))
    return 0


if __name__ == "__main__":
    main()
