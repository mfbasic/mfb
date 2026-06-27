"""Builds a 1000-item list by appending a 10-item list 100 times."""


def main():
    ten = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    nums = []
    for i in range(100):
        nums.extend(ten)
    print("count: " + str(len(nums)) + " last=" + str(nums[len(nums) - 1]))
    return 0


if __name__ == "__main__":
    main()
