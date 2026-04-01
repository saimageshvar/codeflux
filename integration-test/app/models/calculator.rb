class Calculator
  def add(a, b)
    a + b
  end

  def multiply(a, b)
    a * b
  end

  def divide(a, b)
    raise ZeroDivisionError if b == 0
    a.to_f / b
  end
end
