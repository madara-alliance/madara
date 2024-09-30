module.exports = {
  preset: "ts-jest",
  testEnvironment: "node",
  testTimeout: 50 * 60 * 1000,
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        tsconfig: "tsconfig.json",
      },
    ],
  },
  testMatch: ["**/js_tests/**/*.test.ts"],
};
