"""Builds a 1000-item list by prepending one item at a time."""


def main():
    nums = []
    for i in range(1000):
        nums.insert(0, i)
    print("count: " + str(len(nums)) + " first=" + str(nums[0]))
    return 0


if __name__ == "__main__":
    main()
