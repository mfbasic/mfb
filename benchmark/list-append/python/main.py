"""Appends 1000 times to a MUT Integer array and a MUT String array."""


def main():
    nums = []
    for i in range(1000):
        nums.append(i)

    names = []
    for i in range(1000):
        names.append(str(i))

    print("ints: " + str(len(nums)) + " last=" + str(nums[len(nums) - 1]))
    print("strings: " + str(len(names)) + " last=" + names[len(names) - 1])
    return 0


if __name__ == "__main__":
    main()
