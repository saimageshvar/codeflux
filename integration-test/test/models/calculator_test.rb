require_relative "../test_helper"
require "calculator"

class CalculatorTest < Minitest::Test
  def test_add
    calc = Calculator.new
    assert_equal 5, calc.add(2, 3)
  end

  def test_multiply
    calc = Calculator.new
    assert_equal 6, calc.multiply(2, 3)
  end

  def test_divide
    calc = Calculator.new
    assert_in_delta 2.5, calc.divide(5, 2)
  end
end
